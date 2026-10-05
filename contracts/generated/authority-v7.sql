CREATE TABLE meta (id INTEGER PRIMARY KEY CHECK(id=1), generation INTEGER NOT NULL, audit_bytes INTEGER NOT NULL DEFAULT 0);
INSERT INTO meta VALUES (1,0,0);
CREATE TABLE users (id TEXT PRIMARY KEY, username TEXT NOT NULL UNIQUE, password_hash TEXT NOT NULL, role TEXT NOT NULL, grant_revision INTEGER NOT NULL CHECK(grant_revision>0));
CREATE TABLE grants (principal TEXT NOT NULL REFERENCES users(id), operation TEXT NOT NULL, resource TEXT NOT NULL, PRIMARY KEY(principal,operation,resource));
CREATE TABLE sessions (digest TEXT PRIMARY KEY, principal TEXT NOT NULL REFERENCES users(id), csrf_digest TEXT NOT NULL, revision INTEGER NOT NULL, expires INTEGER NOT NULL, created INTEGER NOT NULL);
CREATE INDEX sessions_principal ON sessions(principal,created);
CREATE TABLE bootstrap (id INTEGER PRIMARY KEY CHECK(id=1), digest TEXT NOT NULL, expires INTEGER NOT NULL);
CREATE TABLE resources (id TEXT PRIMARY KEY, kind TEXT NOT NULL, managed INTEGER NOT NULL CHECK(managed IN (0,1)), identity TEXT NOT NULL, revision INTEGER NOT NULL);
CREATE TABLE jobs (id TEXT PRIMARY KEY, principal TEXT NOT NULL REFERENCES users(id), idempotency TEXT NOT NULL, intent TEXT NOT NULL, digest TEXT NOT NULL, resource TEXT NOT NULL, state TEXT NOT NULL CHECK(state IN ('planned','waiting_approval','queued','running','verifying','succeeded','failed','canceled','precondition_changed','outcome_unknown','needs_intervention')), generation INTEGER NOT NULL, revision INTEGER NOT NULL, deadline INTEGER NOT NULL, UNIQUE(principal,idempotency));
CREATE INDEX jobs_state ON jobs(state,deadline);
CREATE UNIQUE INDEX jobs_resource_lock ON jobs(resource) WHERE state IN ('running','verifying','outcome_unknown','needs_intervention');
CREATE TABLE events (cursor INTEGER PRIMARY KEY AUTOINCREMENT, principal TEXT REFERENCES users(id), job TEXT REFERENCES jobs(id), kind TEXT NOT NULL, created INTEGER NOT NULL);
CREATE INDEX events_scope ON events(principal,cursor);
CREATE TABLE task_tokens (digest TEXT PRIMARY KEY, principal TEXT NOT NULL REFERENCES users(id), service_uid INTEGER NOT NULL, task TEXT NOT NULL, scopes TEXT NOT NULL, revision INTEGER NOT NULL, generation INTEGER NOT NULL, expires INTEGER NOT NULL, revoked INTEGER NOT NULL DEFAULT 0 CHECK(revoked IN (0,1)));
CREATE INDEX tasks_principal ON task_tokens(principal,expires);
CREATE TABLE budgets (id TEXT PRIMARY KEY, limit_units INTEGER NOT NULL CHECK(limit_units>=0));
CREATE TABLE reservations (request TEXT PRIMARY KEY, budget TEXT NOT NULL REFERENCES budgets(id), task TEXT NOT NULL, price_revision TEXT NOT NULL, amount INTEGER NOT NULL CHECK(amount>=0), settled INTEGER CHECK(settled>=0 AND settled<=amount));
CREATE INDEX reservations_budget ON reservations(budget);
PRAGMA user_version=1;
CREATE TABLE restart_plans (
    id TEXT PRIMARY KEY,
    principal TEXT NOT NULL REFERENCES users(id),
    body TEXT NOT NULL,
    digest TEXT NOT NULL,
    revision INTEGER NOT NULL,
    expires INTEGER NOT NULL,
    approval_digest TEXT,
    job TEXT UNIQUE REFERENCES jobs(id),
    canceled INTEGER NOT NULL DEFAULT 0 CHECK(canceled IN (0,1))
);
CREATE INDEX restart_plans_owner ON restart_plans(principal,expires);
PRAGMA user_version=2;
CREATE TABLE restart_results (
    job TEXT PRIMARY KEY REFERENCES jobs(id),
    receipt TEXT NOT NULL,
    verification TEXT,
    recorded INTEGER NOT NULL
);
PRAGMA user_version=3;
ALTER TABLE restart_plans RENAME TO container_plans;
ALTER TABLE restart_results RENAME TO container_results;
PRAGMA user_version=4;
CREATE TABLE compose_plans (
    id TEXT PRIMARY KEY,
    principal TEXT NOT NULL REFERENCES users(id),
    body TEXT NOT NULL,
    digest TEXT NOT NULL,
    revision INTEGER NOT NULL,
    expires INTEGER NOT NULL,
    approval_digest TEXT,
    canceled INTEGER NOT NULL DEFAULT 0 CHECK(canceled IN (0,1))
);
CREATE INDEX compose_plans_expiry ON compose_plans(expires);
PRAGMA user_version=5;
CREATE TABLE storage_plans (
    id TEXT PRIMARY KEY,
    principal TEXT NOT NULL REFERENCES users(id),
    body TEXT NOT NULL,
    digest TEXT NOT NULL,
    revision INTEGER NOT NULL,
    expires INTEGER NOT NULL,
    canceled INTEGER NOT NULL DEFAULT 0 CHECK(canceled IN (0,1)),
    approval_digest TEXT
) STRICT;
CREATE INDEX storage_plans_expiry ON storage_plans(expires);
PRAGMA user_version=6;
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
