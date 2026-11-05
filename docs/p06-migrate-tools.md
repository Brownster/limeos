# P06 migrate tools (operator note)

Read-only helpers for the basic cutover. None of them edits the frozen app's
files, and none imports history (SQLite, JSON settings, schedules, backups).

## `limeosctl migrate scan`

Root-only. Walks `/etc/limeos`, `/var/lib/limeos`, `/opt/stacks` (override with
`--etc-dir`, `--state-dir`, `--stacks-dir`; add `--checkout PATH`; `--root DIR`
prefixes all of them, for a copied tree) and lists every file with size, owner,
mode and sha256, classed `imported`, `deferred` or `unsupported` by a closed
table: `credentials.env` is imported; SQLite files, `integrations/`, policy
backups, `auto_update.json`, `backup_config.json`, `package-approvals.json`,
`pending-actions.json` and `storage_plugins/` are deferred; the rest is
unsupported. Names containing `secret` or `env` show size and mode only.
Symlinks, FIFOs and other non-regular files are listed and never opened or
followed. The scan is bounded (20000 files, depth 8, 5000 entries per directory,
256 MiB per hashed file); hitting a bound or an unreadable directory marks the
report incomplete and exits non-zero. Every original is re-checked at the end
and any change fails the run. `--json` emits report schema 1.

## `limeosctl migrate import-users --env-file PATH`

Root-only, and the core accepts the request only from a root peer. Reads
`PIHEALTH_USERS` (or `PIHEALTH_USER` + `PIHEALTH_PASSWORD_HASH`) with the frozen
app's rules: empty or duplicate names and a plaintext `PIHEALTH_PASSWORD` are
refused. Hashes go over the core socket, never argv or logs. One transaction:
any invalid entry imports nothing. A username that already exists is skipped,
never overwritten or compared, and reported as `skipped existing` with its name;
the rest of the batch still imports. A safe re-run (even after users logged in
and their hashes upgraded to Argon2id) therefore changes nothing. Imported users
are administrators.

## `limeosctl migrate propose-ceiling`

`--docker-ps FILE --stacks DIR --core-uid UID --out FILE`. Offline: give it
`docker ps --no-trunc --format json` output (full 64-character IDs are required)
and the stacks directory. It writes a container ceiling with every `allow_*`
false and `managed_containers` set to the listed IDs, refuses to overwrite, and
prints ID/name and stack lists. Review it, install it root-owned, then set only
the flags you intend; `limeosctl check-ceiling` must accept it.
`managed_stacks` lists the stack directories that hold a Compose file (valid
Compose project names only, at most 16) and `allow_stack` is false.
