-- Whole-stack lifecycle. Plans and results mirror container_plans and
-- container_results; jobs, job_resources and resource_locks are reused.
CREATE TABLE stack_plans (
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
CREATE INDEX stack_plans_owner ON stack_plans(principal,expires);
CREATE TABLE stack_results (
    job TEXT PRIMARY KEY REFERENCES jobs(id),
    receipt TEXT NOT NULL,
    verification TEXT,
    recorded INTEGER NOT NULL
);
PRAGMA user_version=15;
