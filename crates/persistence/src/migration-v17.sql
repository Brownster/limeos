-- Per-job event lookups: the startup proofs for every automation job, the
-- per-job progress readers and retention's whole-job deletes. Without it each
-- lookup scanned the table inside the startup transaction.
CREATE INDEX events_job ON events(job,cursor);
-- Lookups by event kind: the health policy revision in force before a cursor
-- (read per observation job, at startup and when queueing), and the startup
-- scans for notification, policy and poll proofs.
CREATE INDEX events_kind ON events(json_extract(kind,'$.kind'),cursor);
PRAGMA user_version=17;
