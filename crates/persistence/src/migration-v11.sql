-- Schedule reservation only. No observation producer or dispatcher is registered.
CREATE TABLE automation_schedules (
    id TEXT PRIMARY KEY CHECK(length(CAST(id AS BLOB))=64),
    owner TEXT NOT NULL REFERENCES users(id),
    policy TEXT NOT NULL CHECK(policy='health-v1'),
    owner_revision INTEGER NOT NULL CHECK(owner_revision BETWEEN 1 AND 9007199254740991),
    revision INTEGER NOT NULL CHECK(revision BETWEEN 1 AND 9007199254740991),
    operation TEXT NOT NULL CHECK(operation IN ('observe_health','backup','protection_sync','protection_scrub','container_update','package_reconcile')),
    definition TEXT NOT NULL CHECK(json_valid(definition) AND length(CAST(definition AS BLOB)) BETWEEN 1 AND 4096),
    paused INTEGER NOT NULL CHECK(paused IN (0,1)),
    settled_through INTEGER CHECK(settled_through BETWEEN 0 AND 253402300799),
    created INTEGER NOT NULL CHECK(created BETWEEN 0 AND 253402300799),
    updated INTEGER NOT NULL CHECK(updated BETWEEN created AND 253402300799),
    CHECK(json_extract(definition,'$.operation')=operation)
) STRICT;
CREATE INDEX automation_schedules_owner ON automation_schedules(owner,id);
CREATE TABLE automation_occurrences (
    schedule TEXT NOT NULL REFERENCES automation_schedules(id),
    revision INTEGER NOT NULL CHECK(revision BETWEEN 1 AND 9007199254740991),
    due INTEGER NOT NULL CHECK(due BETWEEN 0 AND 253402300799),
    owner TEXT NOT NULL REFERENCES users(id),
    policy TEXT NOT NULL CHECK(policy='health-v1'),
    owner_revision INTEGER NOT NULL CHECK(owner_revision BETWEEN 1 AND 9007199254740991),
    settlement TEXT NOT NULL CHECK(settlement IN ('observation','skipped','blocked')),
    job TEXT UNIQUE REFERENCES jobs(id),
    created INTEGER NOT NULL CHECK(created BETWEEN due AND 253402300799),
    PRIMARY KEY(schedule,revision,due),
    CHECK((settlement='observation' AND job IS NOT NULL)
       OR (settlement IN ('skipped','blocked') AND job IS NULL))
) STRICT;
CREATE INDEX automation_occurrences_history ON automation_occurrences(created,schedule,revision,due);
CREATE TRIGGER automation_schedule_identity BEFORE UPDATE ON automation_schedules
WHEN NEW.id IS NOT OLD.id OR NEW.owner IS NOT OLD.owner
  OR NEW.policy IS NOT OLD.policy OR NEW.operation IS NOT OLD.operation
  OR NEW.created IS NOT OLD.created OR NEW.updated<OLD.updated
  OR NEW.revision NOT IN (OLD.revision,OLD.revision+1)
  OR (NEW.revision=OLD.revision AND (NEW.definition IS NOT OLD.definition
      OR NEW.paused IS NOT OLD.paused OR NEW.owner_revision IS NOT OLD.owner_revision
      OR (OLD.settled_through IS NOT NULL AND (NEW.settled_through IS NULL OR NEW.settled_through<OLD.settled_through))))
  OR (NEW.revision<>OLD.revision AND NEW.settled_through IS NOT NULL)
BEGIN SELECT RAISE(ABORT,'immutable automation identity or watermark'); END;
CREATE TRIGGER automation_schedule_delete BEFORE DELETE ON automation_schedules
BEGIN SELECT RAISE(ABORT,'retained automation watermark'); END;
CREATE TRIGGER automation_occurrence_identity BEFORE UPDATE ON automation_occurrences
BEGIN SELECT RAISE(ABORT,'immutable automation occurrence'); END;
-- A later bounded retention owner may prune old detail, never pending work or
-- the current watermark's latest row. Reservation still checks the watermark.
CREATE TRIGGER automation_occurrence_delete BEFORE DELETE ON automation_occurrences
WHEN (OLD.job IS NOT NULL AND (SELECT state FROM jobs WHERE id=OLD.job)
          NOT IN ('succeeded','failed','canceled','precondition_changed'))
  OR NOT EXISTS (SELECT 1 FROM automation_schedules s WHERE s.id=OLD.schedule
      AND (OLD.revision<s.revision OR (OLD.revision=s.revision AND OLD.due<s.settled_through)))
BEGIN SELECT RAISE(ABORT,'retained automation occurrence'); END;
CREATE TRIGGER automation_job_insert_state BEFORE INSERT ON jobs
WHEN json_extract(NEW.intent,'$.operation')='automation_observe_health' AND NEW.state IS NOT 'queued'
BEGIN SELECT RAISE(ABORT,'automation observation is not dispatchable'); END;
CREATE TRIGGER automation_job_update_state BEFORE UPDATE OF state ON jobs
WHEN json_extract(OLD.intent,'$.operation')='automation_observe_health'
  AND (NEW.state NOT IN ('queued','canceled') OR (OLD.state='canceled' AND NEW.state IS NOT OLD.state))
BEGIN SELECT RAISE(ABORT,'automation observation is not dispatchable'); END;
PRAGMA user_version=11;
