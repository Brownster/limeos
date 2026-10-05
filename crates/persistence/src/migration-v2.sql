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
