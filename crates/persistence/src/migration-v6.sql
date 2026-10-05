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
