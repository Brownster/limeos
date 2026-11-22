-- Job-row retention (`retention/jobs.rs`). Until now no job row could be
-- deleted: every job has a resource set, and its primary resource and a
-- completed resolution were immutable. Retention now removes a settled job
-- with everything that names it, in one transaction that defers foreign keys
-- to its commit: the job row first, then its dependents, which may go only
-- once their job is gone. While the job exists they stay exactly as guarded
-- before, and with foreign keys on a job row can be gone while a dependent
-- remains only inside such a transaction, whose commit fails unless every
-- dependent went with it.
DROP TRIGGER IF EXISTS job_resources_delete;
CREATE TRIGGER job_resources_delete BEFORE DELETE ON job_resources
WHEN EXISTS (SELECT 1 FROM jobs WHERE id=OLD.job)
    AND ((SELECT state FROM jobs WHERE id=OLD.job) IS NOT 'queued'
        OR OLD.resource=(SELECT resource FROM jobs WHERE id=OLD.job))
BEGIN SELECT RAISE(ABORT,'immutable resource'); END;

DROP TRIGGER IF EXISTS job_resolutions_delete;
CREATE TRIGGER job_resolutions_delete BEFORE DELETE ON job_resolutions
WHEN OLD.confirmation IS NOT NULL AND EXISTS (SELECT 1 FROM jobs WHERE id=OLD.job)
BEGIN SELECT RAISE(ABORT,'completed resolution is retained'); END;

-- When a job settled, in seconds since the Unix epoch: set by the statement
-- that moves it into a settled state, in that transaction, and never changed
-- after. Retention ages a job by it, since a job's events may be pruned
-- whatever their age. A settled job already stored is taken as settled now,
-- so none of them goes for 30 days. A state change that does not set it
-- (none in core) takes the system clock.
ALTER TABLE jobs ADD COLUMN settled INTEGER;
UPDATE jobs SET settled=unixepoch()
    WHERE state IN ('succeeded','failed','canceled','precondition_changed','resolved');
CREATE TRIGGER jobs_settled BEFORE UPDATE OF state,settled ON jobs
WHEN (OLD.settled IS NOT NULL AND NEW.settled IS NOT OLD.settled)
    OR (NEW.settled IS NOT NULL
        AND NEW.state NOT IN ('succeeded','failed','canceled','precondition_changed','resolved'))
BEGIN SELECT RAISE(ABORT,'settlement time'); END;
CREATE TRIGGER jobs_settled_now AFTER UPDATE OF state ON jobs
WHEN NEW.settled IS NULL
    AND NEW.state IN ('succeeded','failed','canceled','precondition_changed','resolved')
BEGIN UPDATE jobs SET settled=unixepoch() WHERE id=NEW.id; END;

-- Settled jobs by retention's family (`retention::jobs::family`, the same
-- expression: the planner matches it to use this index) and settlement:
-- each family's floor and its candidates, in order. Another operation's jobs
-- have no entry.
CREATE INDEX jobs_retention ON jobs(CASE WHEN json_extract(intent,'$.operation') IN ('container_restart','container_start','container_stop') THEN 0 WHEN json_extract(intent,'$.operation') IN ('stack_restart','stack_start','stack_stop') THEN 1 WHEN json_extract(intent,'$.operation') IN ('health_probe') THEN 2 END,settled)
    WHERE settled IS NOT NULL AND CASE WHEN json_extract(intent,'$.operation') IN ('container_restart','container_start','container_stop') THEN 0 WHEN json_extract(intent,'$.operation') IN ('stack_restart','stack_start','stack_stop') THEN 1 WHEN json_extract(intent,'$.operation') IN ('health_probe') THEN 2 END IS NOT NULL;


-- Only a settled job of an operation retention knows may be deleted, never
-- one with an open resolution request: the same rule as retention's own
-- query, held by the schema too. Its age is not: a trigger has no clock but
-- the system's, which retention does not read.
CREATE TRIGGER jobs_delete BEFORE DELETE ON jobs
WHEN OLD.state NOT IN ('succeeded','failed','canceled','precondition_changed','resolved')
    OR OLD.settled IS NULL
    OR json_extract(OLD.intent,'$.operation') IS NULL
    OR json_extract(OLD.intent,'$.operation') NOT IN ('container_restart','container_start','container_stop','stack_restart','stack_start','stack_stop','health_probe')
    OR EXISTS (SELECT 1 FROM job_resolutions r WHERE r.job=OLD.id AND r.confirmation IS NULL)
BEGIN SELECT RAISE(ABORT,'retained job'); END;
PRAGMA user_version=19;
