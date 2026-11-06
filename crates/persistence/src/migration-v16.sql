-- Operator resolution of needs_intervention jobs. `resolved` is a new terminal,
-- lock-releasing job state; the schema-7 release trigger already deletes the
-- resource_locks rows when a job leaves the lock states. Original jobs, plans,
-- receipts and results are never rewritten: job_resolutions is additional.

-- SQLite cannot alter a CHECK in place and a table rebuild would have to drop
-- every dependent trigger and foreign key. Widening the allowed list changes no
-- stored row and no on-disk format, which is the documented use of
-- writable_schema. RESET reloads the schema for this connection.
PRAGMA writable_schema=ON;
UPDATE sqlite_schema
    SET sql=replace(sql,'''needs_intervention'')','''needs_intervention'',''resolved'')')
    WHERE type='table' AND name='jobs' AND instr(sql,'''resolved''')=0;
PRAGMA writable_schema=RESET;

-- One request per job at a time. `confirmation` (the executor's receipt) and
-- `completed` are set together, once, in the transaction that resolves the job.
CREATE TABLE job_resolutions (
    job TEXT PRIMARY KEY REFERENCES jobs(id),
    resolved_by TEXT NOT NULL REFERENCES users(id),
    declared TEXT NOT NULL CHECK(declared IN ('verified_done','not_done','accepted_divergence')),
    reason TEXT NOT NULL CHECK(
        length(reason) BETWEEN 1 AND 500
        AND (declared<>'accepted_divergence' OR length(reason)>=20)
    ),
    evidence TEXT NOT NULL,
    evidence_digest TEXT NOT NULL,
    requested INTEGER NOT NULL,
    confirmation TEXT,
    completed INTEGER,
    CHECK((confirmation IS NULL)=(completed IS NULL))
) STRICT;

CREATE TRIGGER job_resolutions_insert BEFORE INSERT ON job_resolutions
WHEN (SELECT state FROM jobs WHERE id=NEW.job)
    NOT IN ('running','verifying','needs_intervention')
    OR NEW.confirmation IS NOT NULL
BEGIN SELECT RAISE(ABORT,'job is not awaiting resolution'); END;
CREATE TRIGGER job_resolutions_update BEFORE UPDATE ON job_resolutions
WHEN NEW.job IS NOT OLD.job OR NEW.resolved_by IS NOT OLD.resolved_by
    OR NEW.declared IS NOT OLD.declared OR NEW.reason IS NOT OLD.reason
    OR NEW.evidence IS NOT OLD.evidence OR NEW.evidence_digest IS NOT OLD.evidence_digest
    OR NEW.requested IS NOT OLD.requested OR OLD.confirmation IS NOT NULL
BEGIN SELECT RAISE(ABORT,'immutable resolution'); END;
-- Only an open request can be withdrawn; a completed one is audit history.
CREATE TRIGGER job_resolutions_delete BEFORE DELETE ON job_resolutions
WHEN OLD.confirmation IS NOT NULL
BEGIN SELECT RAISE(ABORT,'completed resolution is retained'); END;

-- A job becomes resolved only from an uncertain state, only with a request
-- whose executor confirmation is recorded, and never leaves resolved.
CREATE TRIGGER jobs_resolved_insert BEFORE INSERT ON jobs
WHEN NEW.state='resolved'
BEGIN SELECT RAISE(ABORT,'resolution required'); END;
CREATE TRIGGER jobs_resolved_update BEFORE UPDATE OF state ON jobs
WHEN (NEW.state='resolved' AND (OLD.state NOT IN ('running','verifying','needs_intervention')
        OR NOT EXISTS(SELECT 1 FROM job_resolutions r WHERE r.job=OLD.id
            AND r.confirmation IS NOT NULL)))
    OR (OLD.state='resolved' AND NEW.state IS NOT 'resolved')
BEGIN SELECT RAISE(ABORT,'resolution required'); END;
PRAGMA user_version=16;
