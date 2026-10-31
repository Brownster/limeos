CREATE TABLE meta (id INTEGER PRIMARY KEY CHECK(id=1), generation INTEGER NOT NULL, audit_bytes INTEGER NOT NULL DEFAULT 0);
INSERT INTO meta VALUES (1,0,0);
CREATE TABLE users (id TEXT PRIMARY KEY, username TEXT NOT NULL UNIQUE, password_hash TEXT NOT NULL, role TEXT NOT NULL, grant_revision INTEGER NOT NULL CHECK(grant_revision>0));
CREATE TABLE grants (principal TEXT NOT NULL REFERENCES users(id), operation TEXT NOT NULL, resource TEXT NOT NULL, PRIMARY KEY(principal,operation,resource));
CREATE TABLE sessions (digest TEXT PRIMARY KEY, principal TEXT NOT NULL REFERENCES users(id), csrf_digest TEXT NOT NULL, revision INTEGER NOT NULL, expires INTEGER NOT NULL, created INTEGER NOT NULL);
CREATE INDEX sessions_principal ON sessions(principal,created);
CREATE TABLE bootstrap (id INTEGER PRIMARY KEY CHECK(id=1), digest TEXT NOT NULL, expires INTEGER NOT NULL);
CREATE TABLE resources (id TEXT PRIMARY KEY, kind TEXT NOT NULL, managed INTEGER NOT NULL CHECK(managed IN (0,1)), identity TEXT NOT NULL, revision INTEGER NOT NULL);
CREATE TABLE jobs (id TEXT PRIMARY KEY, principal TEXT NOT NULL REFERENCES users(id), idempotency TEXT NOT NULL, intent TEXT NOT NULL, digest TEXT NOT NULL, resource TEXT NOT NULL, state TEXT NOT NULL CHECK(state IN ('planned','waiting_approval','queued','running','verifying','succeeded','failed','canceled','precondition_changed','outcome_unknown','needs_intervention')), generation INTEGER NOT NULL, revision INTEGER NOT NULL, deadline INTEGER NOT NULL, UNIQUE(principal,idempotency));
CREATE INDEX jobs_state ON jobs(state,deadline);
CREATE UNIQUE INDEX jobs_resource_lock ON jobs(resource) WHERE state IN ('running','verifying','outcome_unknown','needs_intervention');
CREATE TABLE events (cursor INTEGER PRIMARY KEY AUTOINCREMENT, principal TEXT REFERENCES users(id), job TEXT REFERENCES jobs(id), kind TEXT NOT NULL, created INTEGER NOT NULL);
CREATE INDEX events_scope ON events(principal,cursor);
CREATE TABLE task_tokens (digest TEXT PRIMARY KEY, principal TEXT NOT NULL REFERENCES users(id), service_uid INTEGER NOT NULL, task TEXT NOT NULL, scopes TEXT NOT NULL, revision INTEGER NOT NULL, generation INTEGER NOT NULL, expires INTEGER NOT NULL, revoked INTEGER NOT NULL DEFAULT 0 CHECK(revoked IN (0,1)));
CREATE INDEX tasks_principal ON task_tokens(principal,expires);
CREATE TABLE budgets (id TEXT PRIMARY KEY, limit_units INTEGER NOT NULL CHECK(limit_units>=0));
CREATE TABLE reservations (request TEXT PRIMARY KEY, budget TEXT NOT NULL REFERENCES budgets(id), task TEXT NOT NULL, price_revision TEXT NOT NULL, amount INTEGER NOT NULL CHECK(amount>=0), settled INTEGER CHECK(settled>=0 AND settled<=amount));
CREATE INDEX reservations_budget ON reservations(budget);
PRAGMA user_version=1;
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
CREATE TABLE restart_results (
    job TEXT PRIMARY KEY REFERENCES jobs(id),
    receipt TEXT NOT NULL,
    verification TEXT,
    recorded INTEGER NOT NULL
);
PRAGMA user_version=3;
ALTER TABLE restart_plans RENAME TO container_plans;
ALTER TABLE restart_results RENAME TO container_results;
PRAGMA user_version=4;
CREATE TABLE compose_plans (
    id TEXT PRIMARY KEY,
    principal TEXT NOT NULL REFERENCES users(id),
    body TEXT NOT NULL,
    digest TEXT NOT NULL,
    revision INTEGER NOT NULL,
    expires INTEGER NOT NULL,
    approval_digest TEXT,
    canceled INTEGER NOT NULL DEFAULT 0 CHECK(canceled IN (0,1))
);
CREATE INDEX compose_plans_expiry ON compose_plans(expires);
PRAGMA user_version=5;
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
-- Keep required resources after completion; only active jobs own locks.
CREATE TABLE job_resources (
    job TEXT NOT NULL REFERENCES jobs(id),
    resource TEXT NOT NULL CHECK(
        length(CAST(resource AS BLOB)) BETWEEN 1 AND 768
        AND instr(resource,char(0))=0 AND instr(resource,char(10))=0
        AND instr(resource,char(13))=0
    ),
    PRIMARY KEY(job,resource)
) STRICT;
CREATE TABLE resource_locks (
    resource TEXT PRIMARY KEY,
    job TEXT NOT NULL,
    FOREIGN KEY(job,resource) REFERENCES job_resources(job,resource)
) STRICT;
CREATE INDEX resource_locks_job ON resource_locks(job);

-- Migration preserves every old lock, including uncertain/revoked/expired jobs.
INSERT INTO job_resources SELECT id,resource FROM jobs;
INSERT INTO resource_locks SELECT resource,id FROM jobs
    WHERE state IN ('running','verifying','outcome_unknown','needs_intervention');

CREATE TRIGGER jobs_primary_resource AFTER INSERT ON jobs BEGIN
    INSERT INTO job_resources VALUES(NEW.id,NEW.resource);
END;
CREATE TRIGGER jobs_identity_immutable
BEFORE UPDATE OF id,principal,idempotency,intent,digest,resource,revision,deadline ON jobs
WHEN NEW.id IS NOT OLD.id OR NEW.principal IS NOT OLD.principal
    OR NEW.idempotency IS NOT OLD.idempotency OR NEW.intent IS NOT OLD.intent
    OR NEW.digest IS NOT OLD.digest OR NEW.resource IS NOT OLD.resource
    OR NEW.revision IS NOT OLD.revision OR NEW.deadline IS NOT OLD.deadline
BEGIN SELECT RAISE(ABORT,'immutable job'); END;

-- Operation-owned dependencies are added inside the queue transaction.
-- They cannot be edited after dispatch, even following restart or uncertainty.
CREATE TRIGGER job_resources_insert BEFORE INSERT ON job_resources
WHEN (SELECT state FROM jobs WHERE id=NEW.job) IS NOT 'queued'
    OR (SELECT count(*) FROM job_resources WHERE job=NEW.job)>=128
BEGIN SELECT RAISE(ABORT,'resource set unavailable'); END;
CREATE TRIGGER job_resources_update BEFORE UPDATE ON job_resources
BEGIN SELECT RAISE(ABORT,'immutable resource'); END;
CREATE TRIGGER job_resources_delete BEFORE DELETE ON job_resources
WHEN (SELECT state FROM jobs WHERE id=OLD.job) IS NOT 'queued'
    OR OLD.resource=(SELECT resource FROM jobs WHERE id=OLD.job)
BEGIN SELECT RAISE(ABORT,'immutable resource'); END;

-- A conflicting dependency aborts the entire dispatch state update. There is
-- no partial acquisition, timeout, lease, fencing token or automatic stealing.
CREATE TRIGGER jobs_acquire_resources AFTER UPDATE OF state ON jobs
WHEN NEW.state IN ('running','verifying','outcome_unknown','needs_intervention')
    AND OLD.state NOT IN ('running','verifying','outcome_unknown','needs_intervention')
BEGIN
    SELECT CASE WHEN NOT EXISTS(
        SELECT 1 FROM job_resources WHERE job=NEW.id AND resource=NEW.resource
    ) THEN RAISE(ABORT,'missing primary resource') END;
    INSERT INTO resource_locks SELECT resource,job FROM job_resources
        WHERE job=NEW.id ORDER BY resource;
END;
CREATE TRIGGER jobs_release_resources AFTER UPDATE OF state ON jobs
WHEN NEW.state NOT IN ('running','verifying','outcome_unknown','needs_intervention')
    AND OLD.state IN ('running','verifying','outcome_unknown','needs_intervention')
BEGIN DELETE FROM resource_locks WHERE job=NEW.id; END;

CREATE TRIGGER resource_locks_insert BEFORE INSERT ON resource_locks
WHEN (SELECT state FROM jobs WHERE id=NEW.job)
    NOT IN ('running','verifying','outcome_unknown','needs_intervention')
BEGIN SELECT RAISE(ABORT,'inactive lock owner'); END;
CREATE TRIGGER resource_locks_update BEFORE UPDATE ON resource_locks
BEGIN SELECT RAISE(ABORT,'immutable lock'); END;
CREATE TRIGGER resource_locks_delete BEFORE DELETE ON resource_locks
WHEN (SELECT state FROM jobs WHERE id=OLD.job)
    IN ('running','verifying','outcome_unknown','needs_intervention')
BEGIN SELECT RAISE(ABORT,'active lock'); END;
PRAGMA user_version=7;
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
-- Durable private preparation claims; no executable or recovered owner exists.
CREATE TABLE backup_preparation_plans (
    id TEXT PRIMARY KEY CHECK(length(CAST(id AS BLOB))=64),
    principal TEXT NOT NULL REFERENCES users(id) CHECK(length(CAST(principal AS BLOB)) BETWEEN 1 AND 128),
    body TEXT NOT NULL CHECK(length(CAST(body AS BLOB)) BETWEEN 1 AND 16384),
    digest TEXT NOT NULL CHECK(length(CAST(digest AS BLOB))=64),
    revision INTEGER NOT NULL CHECK(revision>0),
    expires INTEGER NOT NULL,
    canceled INTEGER NOT NULL DEFAULT 0 CHECK(canceled IN (0,1)),
    approval_digest TEXT CHECK(approval_digest IS NULL OR length(CAST(approval_digest AS BLOB))=64),
    consumed_approval_digest TEXT CHECK(consumed_approval_digest IS NULL OR length(CAST(consumed_approval_digest AS BLOB))=64),
    job TEXT UNIQUE REFERENCES jobs(id) CHECK(job IS NULL OR length(CAST(job AS BLOB))=64),
    CHECK((job IS NULL AND consumed_approval_digest IS NULL) OR
          (job IS NOT NULL AND canceled=0 AND approval_digest IS NOT NULL
           AND consumed_approval_digest IS NOT NULL AND consumed_approval_digest=approval_digest))
) STRICT;
CREATE INDEX backup_preparation_plans_expiry ON backup_preparation_plans(expires);
CREATE TRIGGER backup_preparation_plan_immutable
BEFORE UPDATE ON backup_preparation_plans
WHEN NEW.id IS NOT OLD.id OR NEW.principal IS NOT OLD.principal
 OR NEW.body IS NOT OLD.body OR NEW.digest IS NOT OLD.digest
 OR NEW.revision IS NOT OLD.revision OR NEW.expires IS NOT OLD.expires
 OR (OLD.job IS NOT NULL AND (NEW.job IS NOT OLD.job
     OR NEW.approval_digest IS NOT OLD.approval_digest
     OR NEW.consumed_approval_digest IS NOT OLD.consumed_approval_digest
     OR NEW.canceled IS NOT OLD.canceled))
 OR (OLD.canceled=1 AND (NEW.canceled IS NOT OLD.canceled
     OR NEW.approval_digest IS NOT OLD.approval_digest OR NEW.job IS NOT OLD.job))
BEGIN SELECT RAISE(ABORT,'immutable backup preparation'); END;
CREATE TRIGGER backup_preparation_insert_state BEFORE INSERT ON jobs
WHEN json_extract(NEW.intent,'$.operation')='backup_prepare_private'
 AND NEW.state IS NOT 'queued'
BEGIN SELECT RAISE(ABORT,'backup preparation is not dispatchable'); END;
CREATE TRIGGER backup_preparation_update_state BEFORE UPDATE OF state ON jobs
WHEN json_extract(OLD.intent,'$.operation')='backup_prepare_private'
 AND (NEW.state NOT IN ('queued','canceled')
      OR (OLD.state='canceled' AND NEW.state IS NOT OLD.state))
BEGIN SELECT RAISE(ABORT,'backup preparation is not dispatchable'); END;
PRAGMA user_version=9;
-- A fresh executable mount approval never reuses a directory-preparation token.
CREATE TABLE storage_mount_plans (
    id TEXT PRIMARY KEY,
    principal TEXT NOT NULL REFERENCES users(id),
    body TEXT NOT NULL CHECK(length(CAST(body AS BLOB)) BETWEEN 1 AND 24576 AND json_valid(body)),
    digest TEXT NOT NULL CHECK(length(digest)=64),
    revision INTEGER NOT NULL CHECK(revision>0),
    expires INTEGER NOT NULL,
    canceled INTEGER NOT NULL DEFAULT 0 CHECK(canceled IN (0,1)),
    approval_digest TEXT CHECK(approval_digest IS NULL OR length(approval_digest)=64),
    job TEXT UNIQUE REFERENCES jobs(id),
    CHECK(job IS NULL OR (canceled=0 AND approval_digest IS NOT NULL))
) STRICT;
CREATE INDEX storage_mount_plans_expiry ON storage_mount_plans(expires);
CREATE TABLE storage_mount_results (
    job TEXT PRIMARY KEY REFERENCES jobs(id),
    receipt TEXT NOT NULL CHECK(length(CAST(receipt AS BLOB)) BETWEEN 1 AND 65536 AND json_valid(receipt)),
    verification TEXT CHECK(verification IS NULL OR (length(CAST(verification AS BLOB))<=65536 AND json_valid(verification))),
    recorded INTEGER NOT NULL CHECK(recorded>=0)
) STRICT;
CREATE TRIGGER storage_mount_plan_identity
BEFORE UPDATE OF id,principal,body,digest,revision,expires ON storage_mount_plans
WHEN NEW.id IS NOT OLD.id OR NEW.principal IS NOT OLD.principal
    OR NEW.body IS NOT OLD.body OR NEW.digest IS NOT OLD.digest
    OR NEW.revision IS NOT OLD.revision OR NEW.expires IS NOT OLD.expires
BEGIN SELECT RAISE(ABORT,'immutable mount proposal'); END;
CREATE TRIGGER storage_mount_plan_consumed
BEFORE UPDATE OF canceled,approval_digest,job ON storage_mount_plans
WHEN OLD.job IS NOT NULL AND (NEW.canceled IS NOT OLD.canceled
    OR NEW.approval_digest IS NOT OLD.approval_digest OR NEW.job IS NOT OLD.job)
BEGIN SELECT RAISE(ABORT,'consumed mount proposal'); END;
CREATE TRIGGER storage_mount_plan_delete BEFORE DELETE ON storage_mount_plans
WHEN OLD.job IS NOT NULL
BEGIN SELECT RAISE(ABORT,'retained mount proposal'); END;
CREATE TRIGGER storage_mount_result_identity BEFORE UPDATE OF job ON storage_mount_results
WHEN NEW.job IS NOT OLD.job
BEGIN SELECT RAISE(ABORT,'immutable mount result identity'); END;
-- The operation-owned result and independent verification precede release in
-- the same transaction. Prepared/unknown receipts never release the claim set.
CREATE TRIGGER storage_mount_terminal_result BEFORE UPDATE OF state ON jobs
WHEN json_extract(NEW.intent,'$.operation')='storage_mount'
    AND OLD.state IN ('running','verifying','outcome_unknown','needs_intervention')
    AND NEW.state IN ('succeeded','precondition_changed')
BEGIN
    SELECT CASE WHEN NOT EXISTS (
        SELECT 1 FROM storage_mount_results r WHERE r.job=NEW.id
        AND r.verification IS NOT NULL
        AND ((NEW.state='succeeded' AND json_extract(r.receipt,'$.state')='verified'
            AND json_extract(r.receipt,'$.quiescence.state')='reaped'
            AND json_extract(r.receipt,'$.quiescence.outcome.status')='exited'
            AND json_extract(r.receipt,'$.quiescence.outcome.code')=0)
          OR (NEW.state='precondition_changed'
            AND json_extract(r.receipt,'$.state')='precondition_changed'
            AND json_extract(r.receipt,'$.quiescence.state') IN ('not_launched','reaped')))
    ) THEN RAISE(ABORT,'mount verification required') END;
END;
PRAGMA user_version=10;
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
-- Immutable configured health policy; old state remains implicit revision zero.
CREATE TABLE automation_health_policies (
    owner TEXT NOT NULL REFERENCES users(id),
    revision INTEGER NOT NULL CHECK(revision BETWEEN 1 AND 9007199254740991),
    body TEXT NOT NULL CHECK(json_valid(body) AND length(CAST(body AS BLOB)) BETWEEN 1 AND 65536),
    digest TEXT NOT NULL CHECK(length(digest)=64 AND digest NOT GLOB '*[^0-9a-f]*'),
    updated INTEGER NOT NULL CHECK(updated BETWEEN 0 AND 253402300799),
    PRIMARY KEY(owner,revision)
) STRICT;
CREATE TRIGGER automation_health_policy_insert BEFORE INSERT ON automation_health_policies
WHEN NEW.revision<>coalesce((SELECT max(revision) FROM automation_health_policies WHERE owner=NEW.owner),0)+1
 OR NEW.updated<coalesce((SELECT max(updated) FROM automation_health_policies WHERE owner=NEW.owner),0)
BEGIN SELECT RAISE(ABORT,'health policy revision or time'); END;
CREATE TRIGGER automation_health_policy_update BEFORE UPDATE ON automation_health_policies
BEGIN SELECT RAISE(ABORT,'immutable health policy'); END;
CREATE TRIGGER automation_health_policy_delete BEFORE DELETE ON automation_health_policies
BEGIN SELECT RAISE(ABORT,'retained health policy'); END;
ALTER TABLE automation_occurrences ADD COLUMN policy_revision INTEGER NOT NULL DEFAULT 0 CHECK(policy_revision BETWEEN 0 AND 9007199254740991);
ALTER TABLE automation_observation_results ADD COLUMN policy_revision INTEGER NOT NULL DEFAULT 0 CHECK(policy_revision BETWEEN 0 AND 9007199254740991);
ALTER TABLE automation_streams ADD COLUMN policy_revision INTEGER NOT NULL DEFAULT 0 CHECK(policy_revision BETWEEN 0 AND 9007199254740991);
CREATE TRIGGER automation_result_policy BEFORE INSERT ON automation_observation_results
WHEN NEW.policy_revision IS NOT (SELECT policy_revision FROM automation_occurrences WHERE job=NEW.job)
BEGIN SELECT RAISE(ABORT,'observation policy binding required'); END;
PRAGMA user_version=13;
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
