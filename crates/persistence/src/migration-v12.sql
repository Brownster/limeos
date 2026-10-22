-- Read-only observation settlement; generic dispatch remains forbidden.
CREATE TABLE automation_streams (
    owner TEXT PRIMARY KEY REFERENCES users(id),
    sequence INTEGER NOT NULL CHECK(sequence BETWEEN 1 AND 9007199254740991),
    complete INTEGER NOT NULL CHECK(complete IN (0,1)),
    owner_revision INTEGER NOT NULL CHECK(owner_revision BETWEEN 1 AND 9007199254740991),
    recorded INTEGER NOT NULL CHECK(recorded BETWEEN 0 AND 253402300799),
    alerts_digest TEXT NOT NULL CHECK(length(alerts_digest)=64 AND alerts_digest NOT GLOB '*[^0-9a-f]*')
) STRICT;
CREATE TABLE automation_observation_results (
    job TEXT PRIMARY KEY REFERENCES jobs(id),
    owner TEXT NOT NULL REFERENCES users(id),
    sequence INTEGER NOT NULL CHECK(sequence BETWEEN 1 AND 9007199254740991),
    owner_revision INTEGER NOT NULL CHECK(owner_revision BETWEEN 1 AND 9007199254740991),
    outcome TEXT NOT NULL CHECK(outcome IN ('succeeded','failed','precondition_changed')),
    reason TEXT NOT NULL CHECK(reason IN ('observed','expired','authority_changed')),
    complete INTEGER NOT NULL CHECK(complete IN (0,1)),
    digest TEXT NOT NULL CHECK(length(digest)=64 AND digest NOT GLOB '*[^0-9a-f]*'),
    recorded INTEGER NOT NULL CHECK(recorded BETWEEN 0 AND 253402300799),
    notifications INTEGER NOT NULL CHECK(notifications BETWEEN 0 AND 256),
    alerts_digest TEXT NOT NULL CHECK(length(alerts_digest)=64 AND alerts_digest NOT GLOB '*[^0-9a-f]*'),
    UNIQUE(owner,sequence),
    CHECK((reason='observed' AND outcome='succeeded')
       OR (reason='expired' AND outcome='failed' AND complete=0)
       OR (reason='authority_changed' AND outcome='precondition_changed' AND complete=0))
) STRICT;
CREATE TABLE automation_alerts (
    owner TEXT NOT NULL REFERENCES users(id),
    key TEXT NOT NULL CHECK(length(CAST(key AS BLOB)) BETWEEN 1 AND 256),
    resource TEXT NOT NULL CHECK(length(CAST(resource AS BLOB)) BETWEEN 1 AND 512),
    body TEXT NOT NULL CHECK(json_valid(body) AND length(CAST(body AS BLOB)) BETWEEN 1 AND 4096),
    updated INTEGER NOT NULL CHECK(updated BETWEEN 0 AND 253402300799),
    PRIMARY KEY(owner,key),
    CHECK(json_extract(body,'$.key') IS key)
) STRICT;
CREATE TABLE automation_notifications (
    id TEXT PRIMARY KEY CHECK(length(id)=64 AND id NOT GLOB '*[^0-9a-f]*'),
    owner TEXT NOT NULL REFERENCES users(id),
    owner_revision INTEGER NOT NULL CHECK(owner_revision BETWEEN 1 AND 9007199254740991),
    resource TEXT NOT NULL CHECK(length(CAST(resource AS BLOB)) BETWEEN 1 AND 512),
    job TEXT NOT NULL REFERENCES automation_observation_results(job) DEFERRABLE INITIALLY DEFERRED,
    key TEXT NOT NULL CHECK(length(CAST(key AS BLOB)) BETWEEN 1 AND 256),
    generation INTEGER NOT NULL CHECK(generation BETWEEN 1 AND 9007199254740991),
    phase TEXT NOT NULL CHECK(phase IN ('incident','recovery')),
    body TEXT NOT NULL CHECK(json_valid(body) AND length(CAST(body AS BLOB)) BETWEEN 1 AND 4096),
    UNIQUE(owner,key,generation,phase),
    CHECK(json_extract(body,'$.id') IS id),
    CHECK(json_extract(body,'$.event.key') IS key),
    CHECK(json_extract(body,'$.event.generation') IS generation),
    CHECK(json_extract(body,'$.event.event') IS phase),
    CHECK(json_extract(body,'$.channel') IS 'in_app'),
    CHECK(json_extract(body,'$.delivery.state') IN ('pending','delivered'))
) STRICT;
CREATE INDEX automation_notifications_owner ON automation_notifications(owner,id);
CREATE TRIGGER automation_stream_identity BEFORE UPDATE ON automation_streams
WHEN NEW.owner IS NOT OLD.owner OR NEW.sequence<>OLD.sequence+1 OR NEW.recorded<OLD.recorded
BEGIN SELECT RAISE(ABORT,'immutable observation sequence'); END;
CREATE TRIGGER automation_stream_delete BEFORE DELETE ON automation_streams
BEGIN SELECT RAISE(ABORT,'retained observation sequence'); END;
CREATE TRIGGER automation_result_insert BEFORE INSERT ON automation_observation_results
WHEN NOT EXISTS (SELECT 1 FROM jobs j WHERE j.id=NEW.job AND j.principal=NEW.owner
    AND j.revision=NEW.owner_revision AND j.state='queued'
    AND json_extract(j.intent,'$.operation')='automation_observe_health')
 OR NEW.sequence<>coalesce((SELECT sequence FROM automation_streams WHERE owner=NEW.owner),0)+1
BEGIN SELECT RAISE(ABORT,'observation settlement requires queued job and next sequence'); END;
CREATE TRIGGER automation_result_update BEFORE UPDATE ON automation_observation_results
BEGIN SELECT RAISE(ABORT,'immutable observation result'); END;
CREATE TRIGGER automation_result_delete BEFORE DELETE ON automation_observation_results
BEGIN SELECT RAISE(ABORT,'retained observation result'); END;
CREATE TRIGGER automation_alert_identity BEFORE UPDATE ON automation_alerts
WHEN NEW.owner IS NOT OLD.owner OR NEW.key IS NOT OLD.key OR NEW.resource IS NOT OLD.resource
 OR NEW.updated<OLD.updated OR json_extract(NEW.body,'$.last_sequence')<=json_extract(OLD.body,'$.last_sequence')
BEGIN SELECT RAISE(ABORT,'immutable alert binding or sequence'); END;
CREATE TRIGGER automation_alert_delete BEFORE DELETE ON automation_alerts
BEGIN SELECT RAISE(ABORT,'current alert material is retained'); END;
CREATE TRIGGER automation_notification_identity BEFORE UPDATE ON automation_notifications
WHEN NEW.id IS NOT OLD.id OR NEW.owner IS NOT OLD.owner OR NEW.resource IS NOT OLD.resource
 OR NEW.owner_revision IS NOT OLD.owner_revision OR NEW.job IS NOT OLD.job
 OR NEW.key IS NOT OLD.key OR NEW.generation IS NOT OLD.generation OR NEW.phase IS NOT OLD.phase
 OR json_extract(NEW.body,'$.event') IS NOT json_extract(OLD.body,'$.event')
 OR json_extract(NEW.body,'$.revision')<=json_extract(OLD.body,'$.revision')
BEGIN SELECT RAISE(ABORT,'immutable notification binding or revision'); END;
CREATE TRIGGER automation_notification_delete BEFORE DELETE ON automation_notifications
WHEN json_extract(OLD.body,'$.delivery.state') IS NOT 'delivered'
 OR (OLD.phase='incident' AND (
    EXISTS (SELECT 1 FROM automation_notifications n WHERE n.owner=OLD.owner AND n.key=OLD.key
        AND n.generation=OLD.generation AND n.phase='recovery')
    OR EXISTS (SELECT 1 FROM automation_alerts a WHERE a.owner=OLD.owner AND a.key=OLD.key
        AND json_extract(a.body,'$.incident.phase')='open'
        AND json_extract(a.body,'$.incident.generation')=OLD.generation)))
BEGIN SELECT RAISE(ABORT,'notification receipt is retained'); END;
DROP TRIGGER automation_job_update_state;
CREATE TRIGGER automation_job_update_state BEFORE UPDATE OF state ON jobs
WHEN json_extract(OLD.intent,'$.operation')='automation_observe_health'
 AND NOT ((NEW.state=OLD.state AND NEW.state IN ('queued','canceled','succeeded','failed','precondition_changed'))
    OR (OLD.state='queued' AND NEW.state='canceled')
    OR (OLD.state='queued' AND NEW.state IN ('succeeded','failed','precondition_changed')
        AND EXISTS (SELECT 1 FROM automation_observation_results r WHERE r.job=OLD.id
            AND r.owner=OLD.principal AND r.owner_revision=OLD.revision AND r.outcome=NEW.state)))
BEGIN SELECT RAISE(ABORT,'observation result required; generic dispatch forbidden'); END;
PRAGMA user_version=12;
