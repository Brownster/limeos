-- Durable private preparation claims; no executable or recovered owner exists.
CREATE TABLE backup_preparation_plans (
    id TEXT PRIMARY KEY CHECK(length(CAST(id AS BLOB))=64),
    principal TEXT NOT NULL REFERENCES users(id) CHECK(length(CAST(principal AS BLOB)) BETWEEN 1 AND 128),
    body TEXT NOT NULL CHECK(length(CAST(body AS BLOB)) BETWEEN 1 AND 16384),
    digest TEXT NOT NULL CHECK(length(CAST(digest AS BLOB))=64),
    revision INTEGER NOT NULL CHECK(revision>0),
    expires INTEGER NOT NULL,
    canceled INTEGER NOT NULL DEFAULT 0 CHECK(canceled IN (0,1)),
    approval_digest TEXT CHECK(approval_digest IS NULL OR length(CAST(approval_digest AS BLOB))=64),
    consumed_approval_digest TEXT CHECK(consumed_approval_digest IS NULL OR length(CAST(consumed_approval_digest AS BLOB))=64),
    job TEXT UNIQUE REFERENCES jobs(id) CHECK(job IS NULL OR length(CAST(job AS BLOB))=64),
    CHECK((job IS NULL AND consumed_approval_digest IS NULL) OR
          (job IS NOT NULL AND canceled=0 AND approval_digest IS NOT NULL
           AND consumed_approval_digest IS NOT NULL AND consumed_approval_digest=approval_digest))
) STRICT;
CREATE INDEX backup_preparation_plans_expiry ON backup_preparation_plans(expires);
CREATE TRIGGER backup_preparation_plan_immutable
BEFORE UPDATE ON backup_preparation_plans
WHEN NEW.id IS NOT OLD.id OR NEW.principal IS NOT OLD.principal
 OR NEW.body IS NOT OLD.body OR NEW.digest IS NOT OLD.digest
 OR NEW.revision IS NOT OLD.revision OR NEW.expires IS NOT OLD.expires
 OR (OLD.job IS NOT NULL AND (NEW.job IS NOT OLD.job
     OR NEW.approval_digest IS NOT OLD.approval_digest
     OR NEW.consumed_approval_digest IS NOT OLD.consumed_approval_digest
     OR NEW.canceled IS NOT OLD.canceled))
 OR (OLD.canceled=1 AND (NEW.canceled IS NOT OLD.canceled
     OR NEW.approval_digest IS NOT OLD.approval_digest OR NEW.job IS NOT OLD.job))
BEGIN SELECT RAISE(ABORT,'immutable backup preparation'); END;
CREATE TRIGGER backup_preparation_insert_state BEFORE INSERT ON jobs
WHEN json_extract(NEW.intent,'$.operation')='backup_prepare_private'
 AND NEW.state IS NOT 'queued'
BEGIN SELECT RAISE(ABORT,'backup preparation is not dispatchable'); END;
CREATE TRIGGER backup_preparation_update_state BEFORE UPDATE OF state ON jobs
WHEN json_extract(OLD.intent,'$.operation')='backup_prepare_private'
 AND (NEW.state NOT IN ('queued','canceled')
      OR (OLD.state='canceled' AND NEW.state IS NOT OLD.state))
BEGIN SELECT RAISE(ABORT,'backup preparation is not dispatchable'); END;
PRAGMA user_version=9;
