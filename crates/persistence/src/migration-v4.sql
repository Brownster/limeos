ALTER TABLE restart_plans RENAME TO container_plans;
ALTER TABLE restart_results RENAME TO container_results;
PRAGMA user_version=4;
