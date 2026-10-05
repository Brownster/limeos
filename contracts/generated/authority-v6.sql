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
