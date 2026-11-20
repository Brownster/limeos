-- The container executor's receipt-store epoch core last saw (ADR 0003). The
-- executor draws a new epoch whenever its store is opened, so a store lost
-- and recreated, or restored from an older copy, answers under another one.
-- When an answer carries an epoch other than this, every held container and
-- stack job gets an executor_store_changed event, and core no longer accepts
-- a never_recorded closure for it. NULL until the first answer that carries
-- one; nothing else changes.
ALTER TABLE meta ADD COLUMN executor_epoch TEXT;
PRAGMA user_version=18;
