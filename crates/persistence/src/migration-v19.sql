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

-- Only a settled job of an operation retention knows may be deleted, never
-- one with an open resolution request: the same rule as retention's own
-- query, held by the schema too.
CREATE TRIGGER jobs_delete BEFORE DELETE ON jobs
WHEN OLD.state NOT IN ('succeeded','failed','canceled','precondition_changed','resolved')
    OR json_extract(OLD.intent,'$.operation') IS NULL
    OR json_extract(OLD.intent,'$.operation') NOT IN ('container_restart','container_start','container_stop','stack_restart','stack_start','stack_stop','health_probe')
    OR EXISTS (SELECT 1 FROM job_resolutions r WHERE r.job=OLD.id AND r.confirmation IS NULL)
BEGIN SELECT RAISE(ABORT,'retained job'); END;
PRAGMA user_version=19;
