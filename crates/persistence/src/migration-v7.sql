-- Keep required resources after completion; only active jobs own locks.
CREATE TABLE job_resources (
    job TEXT NOT NULL REFERENCES jobs(id),
    resource TEXT NOT NULL CHECK(
        length(CAST(resource AS BLOB)) BETWEEN 1 AND 768
        AND instr(resource,char(0))=0 AND instr(resource,char(10))=0
        AND instr(resource,char(13))=0
    ),
    PRIMARY KEY(job,resource)
) STRICT;
CREATE TABLE resource_locks (
    resource TEXT PRIMARY KEY,
    job TEXT NOT NULL,
    FOREIGN KEY(job,resource) REFERENCES job_resources(job,resource)
) STRICT;
CREATE INDEX resource_locks_job ON resource_locks(job);

-- Migration preserves every old lock, including uncertain/revoked/expired jobs.
INSERT INTO job_resources SELECT id,resource FROM jobs;
INSERT INTO resource_locks SELECT resource,id FROM jobs
    WHERE state IN ('running','verifying','outcome_unknown','needs_intervention');

CREATE TRIGGER jobs_primary_resource AFTER INSERT ON jobs BEGIN
    INSERT INTO job_resources VALUES(NEW.id,NEW.resource);
END;
CREATE TRIGGER jobs_identity_immutable
BEFORE UPDATE OF id,principal,idempotency,intent,digest,resource,revision,deadline ON jobs
WHEN NEW.id IS NOT OLD.id OR NEW.principal IS NOT OLD.principal
    OR NEW.idempotency IS NOT OLD.idempotency OR NEW.intent IS NOT OLD.intent
    OR NEW.digest IS NOT OLD.digest OR NEW.resource IS NOT OLD.resource
    OR NEW.revision IS NOT OLD.revision OR NEW.deadline IS NOT OLD.deadline
BEGIN SELECT RAISE(ABORT,'immutable job'); END;

-- Operation-owned dependencies are added inside the queue transaction.
-- They cannot be edited after dispatch, even following restart or uncertainty.
CREATE TRIGGER job_resources_insert BEFORE INSERT ON job_resources
WHEN (SELECT state FROM jobs WHERE id=NEW.job) IS NOT 'queued'
    OR (SELECT count(*) FROM job_resources WHERE job=NEW.job)>=128
BEGIN SELECT RAISE(ABORT,'resource set unavailable'); END;
CREATE TRIGGER job_resources_update BEFORE UPDATE ON job_resources
BEGIN SELECT RAISE(ABORT,'immutable resource'); END;
CREATE TRIGGER job_resources_delete BEFORE DELETE ON job_resources
WHEN (SELECT state FROM jobs WHERE id=OLD.job) IS NOT 'queued'
    OR OLD.resource=(SELECT resource FROM jobs WHERE id=OLD.job)
BEGIN SELECT RAISE(ABORT,'immutable resource'); END;

-- A conflicting dependency aborts the entire dispatch state update. There is
-- no partial acquisition, timeout, lease, fencing token or automatic stealing.
CREATE TRIGGER jobs_acquire_resources AFTER UPDATE OF state ON jobs
WHEN NEW.state IN ('running','verifying','outcome_unknown','needs_intervention')
    AND OLD.state NOT IN ('running','verifying','outcome_unknown','needs_intervention')
BEGIN
    SELECT CASE WHEN NOT EXISTS(
        SELECT 1 FROM job_resources WHERE job=NEW.id AND resource=NEW.resource
    ) THEN RAISE(ABORT,'missing primary resource') END;
    INSERT INTO resource_locks SELECT resource,job FROM job_resources
        WHERE job=NEW.id ORDER BY resource;
END;
CREATE TRIGGER jobs_release_resources AFTER UPDATE OF state ON jobs
WHEN NEW.state NOT IN ('running','verifying','outcome_unknown','needs_intervention')
    AND OLD.state IN ('running','verifying','outcome_unknown','needs_intervention')
BEGIN DELETE FROM resource_locks WHERE job=NEW.id; END;

CREATE TRIGGER resource_locks_insert BEFORE INSERT ON resource_locks
WHEN (SELECT state FROM jobs WHERE id=NEW.job)
    NOT IN ('running','verifying','outcome_unknown','needs_intervention')
BEGIN SELECT RAISE(ABORT,'inactive lock owner'); END;
CREATE TRIGGER resource_locks_update BEFORE UPDATE ON resource_locks
BEGIN SELECT RAISE(ABORT,'immutable lock'); END;
CREATE TRIGGER resource_locks_delete BEFORE DELETE ON resource_locks
WHEN (SELECT state FROM jobs WHERE id=OLD.job)
    IN ('running','verifying','outcome_unknown','needs_intervention')
BEGIN SELECT RAISE(ABORT,'active lock'); END;
PRAGMA user_version=7;
