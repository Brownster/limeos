-- A fresh executable mount approval never reuses a directory-preparation token.
CREATE TABLE storage_mount_plans (
    id TEXT PRIMARY KEY,
    principal TEXT NOT NULL REFERENCES users(id),
    body TEXT NOT NULL CHECK(length(CAST(body AS BLOB)) BETWEEN 1 AND 24576 AND json_valid(body)),
    digest TEXT NOT NULL CHECK(length(digest)=64),
    revision INTEGER NOT NULL CHECK(revision>0),
    expires INTEGER NOT NULL,
    canceled INTEGER NOT NULL DEFAULT 0 CHECK(canceled IN (0,1)),
    approval_digest TEXT CHECK(approval_digest IS NULL OR length(approval_digest)=64),
    job TEXT UNIQUE REFERENCES jobs(id),
    CHECK(job IS NULL OR (canceled=0 AND approval_digest IS NOT NULL))
) STRICT;
CREATE INDEX storage_mount_plans_expiry ON storage_mount_plans(expires);
CREATE TABLE storage_mount_results (
    job TEXT PRIMARY KEY REFERENCES jobs(id),
    receipt TEXT NOT NULL CHECK(length(CAST(receipt AS BLOB)) BETWEEN 1 AND 65536 AND json_valid(receipt)),
    verification TEXT CHECK(verification IS NULL OR (length(CAST(verification AS BLOB))<=65536 AND json_valid(verification))),
    recorded INTEGER NOT NULL CHECK(recorded>=0)
) STRICT;
CREATE TRIGGER storage_mount_plan_identity
BEFORE UPDATE OF id,principal,body,digest,revision,expires ON storage_mount_plans
WHEN NEW.id IS NOT OLD.id OR NEW.principal IS NOT OLD.principal
    OR NEW.body IS NOT OLD.body OR NEW.digest IS NOT OLD.digest
    OR NEW.revision IS NOT OLD.revision OR NEW.expires IS NOT OLD.expires
BEGIN SELECT RAISE(ABORT,'immutable mount proposal'); END;
CREATE TRIGGER storage_mount_plan_consumed
BEFORE UPDATE OF canceled,approval_digest,job ON storage_mount_plans
WHEN OLD.job IS NOT NULL AND (NEW.canceled IS NOT OLD.canceled
    OR NEW.approval_digest IS NOT OLD.approval_digest OR NEW.job IS NOT OLD.job)
BEGIN SELECT RAISE(ABORT,'consumed mount proposal'); END;
CREATE TRIGGER storage_mount_plan_delete BEFORE DELETE ON storage_mount_plans
WHEN OLD.job IS NOT NULL
BEGIN SELECT RAISE(ABORT,'retained mount proposal'); END;
CREATE TRIGGER storage_mount_result_identity BEFORE UPDATE OF job ON storage_mount_results
WHEN NEW.job IS NOT OLD.job
BEGIN SELECT RAISE(ABORT,'immutable mount result identity'); END;
-- The operation-owned result and independent verification precede release in
-- the same transaction. Prepared/unknown receipts never release the claim set.
CREATE TRIGGER storage_mount_terminal_result BEFORE UPDATE OF state ON jobs
WHEN json_extract(NEW.intent,'$.operation')='storage_mount'
    AND OLD.state IN ('running','verifying','outcome_unknown','needs_intervention')
    AND NEW.state IN ('succeeded','precondition_changed')
BEGIN
    SELECT CASE WHEN NOT EXISTS (
        SELECT 1 FROM storage_mount_results r WHERE r.job=NEW.id
        AND r.verification IS NOT NULL
        AND ((NEW.state='succeeded' AND json_extract(r.receipt,'$.state')='verified'
            AND json_extract(r.receipt,'$.quiescence.state')='reaped'
            AND json_extract(r.receipt,'$.quiescence.outcome.status')='exited'
            AND json_extract(r.receipt,'$.quiescence.outcome.code')=0)
          OR (NEW.state='precondition_changed'
            AND json_extract(r.receipt,'$.state')='precondition_changed'
            AND json_extract(r.receipt,'$.quiescence.state') IN ('not_launched','reaped')))
    ) THEN RAISE(ABORT,'mount verification required') END;
END;
PRAGMA user_version=10;
