CREATE TABLE restart_results (
    job TEXT PRIMARY KEY REFERENCES jobs(id),
    receipt TEXT NOT NULL,
    verification TEXT,
    recorded INTEGER NOT NULL
);
PRAGMA user_version=3;
