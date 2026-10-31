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
