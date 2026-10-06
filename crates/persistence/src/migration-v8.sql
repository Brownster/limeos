-- Executable target preparation is distinct from schema-6 preview approvals.
CREATE TABLE storage_target_plans (
    id TEXT PRIMARY KEY,
    principal TEXT NOT NULL REFERENCES users(id),
    body TEXT NOT NULL,
    digest TEXT NOT NULL,
    revision INTEGER NOT NULL,
    expires INTEGER NOT NULL,
    canceled INTEGER NOT NULL DEFAULT 0 CHECK(canceled IN (0,1)),
    approval_digest TEXT,
    job TEXT UNIQUE REFERENCES jobs(id)
) STRICT;
CREATE INDEX storage_target_plans_expiry ON storage_target_plans(expires);
CREATE TABLE storage_target_results (
    job TEXT PRIMARY KEY REFERENCES jobs(id),
    receipt TEXT NOT NULL,
    verification TEXT,
    recorded INTEGER NOT NULL
) STRICT;
PRAGMA user_version=8;
