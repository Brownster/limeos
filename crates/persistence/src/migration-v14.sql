-- Read-only polls become durable jobs only when material changes.
CREATE TABLE automation_poll_origins (
    job TEXT PRIMARY KEY REFERENCES jobs(id),
    definition TEXT NOT NULL CHECK(json_valid(definition) AND length(CAST(definition AS BLOB)) BETWEEN 1 AND 4096),
    admitted INTEGER NOT NULL CHECK(admitted BETWEEN 0 AND 253402300799),
    previous_due INTEGER CHECK(previous_due BETWEEN 0 AND 253402300799),
    lease_digest TEXT NOT NULL CHECK(length(lease_digest)=64 AND lease_digest NOT GLOB '*[^0-9a-f]*'),
    FOREIGN KEY(job) REFERENCES automation_observation_results(job) DEFERRABLE INITIALLY DEFERRED
) STRICT;
CREATE TRIGGER automation_poll_origin_insert BEFORE INSERT ON automation_poll_origins
WHEN NOT EXISTS (SELECT 1 FROM jobs WHERE id=NEW.job AND state='queued'
    AND json_extract(intent,'$.operation')='automation_observe_health')
BEGIN SELECT RAISE(ABORT,'poll origin requires observation job'); END;
CREATE TRIGGER automation_poll_origin_update BEFORE UPDATE ON automation_poll_origins
BEGIN SELECT RAISE(ABORT,'immutable poll origin'); END;
CREATE TRIGGER automation_poll_origin_delete BEFORE DELETE ON automation_poll_origins
BEGIN SELECT RAISE(ABORT,'retained poll origin'); END;
CREATE TRIGGER automation_polled_result BEFORE INSERT ON automation_observation_results
WHEN EXISTS (SELECT 1 FROM automation_poll_origins WHERE job=NEW.job)
 AND (NEW.reason<>'observed' OR NEW.outcome<>'succeeded'
    OR NEW.recorded<(SELECT admitted FROM automation_poll_origins WHERE job=NEW.job)
    OR NEW.recorded>(SELECT admitted+10 FROM automation_poll_origins WHERE job=NEW.job))
BEGIN SELECT RAISE(ABORT,'polled result requires live successful observation'); END;
PRAGMA user_version=14;
