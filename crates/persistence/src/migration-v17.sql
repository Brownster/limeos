-- Per-job event lookups: the startup proofs for every automation job, the
-- per-job progress readers and retention's whole-job deletes. Without it each
-- lookup scanned the table inside the startup transaction.
CREATE INDEX events_job ON events(job,cursor);
-- Lookups by event kind: the health policy revision in force before a cursor
-- (read per observation job, at startup and when queueing), and the startup
-- scans for notification, policy and poll proofs.
CREATE INDEX events_kind ON events(json_extract(kind,'$.kind'),cursor);
-- Lookups by notification: each live notification's proofs and each pruned
-- one's publication, acknowledgement and prune proofs, at startup. Only
-- notification events have the field, so the index stays small.
CREATE INDEX events_notification ON events(json_extract(kind,'$.notification'),cursor) WHERE json_extract(kind,'$.notification') IS NOT NULL;
PRAGMA user_version=17;
