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
followed. A Python virtual environment is listed as one `virtualenv` entry
(unsupported, no hash) and never descended, whatever its depth, since it is
software and not state: a directory holding a regular `pyvenv.cfg` whose first
4 KiB have a `home = ...` line, and either a `bin/python*` entry or a
`lib/python*/site-packages` directory. A directory that merely holds a file of
that name is walked like any other. The entries under a virtual environment
are counted, without following a link, up to 200,000 entries and 64 levels
(`entries_not_listed`, with `entries_not_listed_bounded` when a bound cut the
count short), and the summary gives the number of virtual environments and of
entries not listed. wybie's `/var/lib/limeos/agent-supervisor/venv` (1,096
files, 74 of its directories past the depth bound) made the scan incomplete on
26 Nov. `scripts/cutover/state_lists.py` builds the cutover's state and live
lists from the JSON report (runbook step 1h) and prints each virtual
environment it left out with its count. The scan is bounded (20000 files, depth 8, 5000 entries per directory,
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
`docker ps --no-trunc --format '{{json .}}'` output (the plain `--format json` keyword needs Docker 23 or newer) (full 64-character IDs are required)
and the stacks directory. It writes a container ceiling with every `allow_*`
false and `managed_containers` set to the listed IDs, refuses to overwrite, and
prints ID/name and stack lists. Review it, install it root-owned, then set only
the flags you intend; `limeosctl check-ceiling FILE CORE_UID container` must accept it.
It runs the container executor's own checks, including the walk: every directory
on the path root-owned and not group- or world-writable, no symlink, the file a
single link of at most 64 KiB. So check the proposal where it will live, or in a
root-owned directory such as `/root`; a copy under `/tmp` is refused, as the
executor would refuse it.
`managed_stacks` lists the stack directories that hold a Compose file (valid
Compose project names only, at most 16) and `allow_stack` is false.

The proposal is deliberately inert, including `allow_health` and `allow_container_read`:
installed verbatim, the executor refuses even its health probe. The operator step sets
at least `allow_health`, `allow_container_read` and, for the approved actions,
`allow_container_logs`, `allow_restart`, `allow_start`, `allow_stop` and `allow_stack`.
Stack status and whole-stack actions also need a root-owned
`/etc/limeos/management-catalog.json`: use `propose-catalog` below, see
[the stack inventory handoff](p04-stack-inventory-runtime-handoff.md).

## `limeosctl migrate propose-catalog`

`--out FILE [--stacks DIR] [--install]`. Root-only; read-only against Docker (`docker info`,
`ps -a -q`, `inspect`, found at `/usr/bin/docker`, `/usr/local/bin/docker` or `/bin/docker`, or
`--docker PATH`; `inspect` emits JSON on every Docker version, so there is no `--format json`
caveat) and the stacks root (default `/opt/stacks`). Offline form for review or tests:
`--docker-inspect FILE --engine-id ID`. It writes a new catalog file (refuses to overwrite) and
prints, per stack, `stack NAME (project P) DIR/COMPOSE services=...`, then every `refused ...`
with its reason, and how many containers it ignored.

- A project is registered when its running or stopped, non-one-off containers carry the Compose
  labels and the working directory is directly under the root. `stack` is the directory name
  and `project` the Compose project label; they may differ. The Compose file (one of the four
  standard names, a regular file of at most 64 KiB, never a symlink) is hashed; an existing
  `limeos.override.json` is recorded with its hash, otherwise `managed` is absent. Services
  are the distinct service labels, so a service that has never had a container is missing:
  create the stack with Compose first.
- Refused with a reason: projects outside the root (wybie's `docker`), several Compose files,
  names the catalog cannot hold (uppercase directory, non-matching project), more than 16
  services, directories with a Compose file but no containers, and any stack past the eighth
  (the first eight by name are kept). If nothing can be registered it exits non-zero and writes
  nothing. One-off containers and containers without a Compose project are ignored and counted.
- Without `--install` it only writes `--out`; install it with `install -o root -g root -m 0644
  FILE /etc/limeos/management-catalog.json`. With `--install` it also links it into place as
  root-owned 0644 after checking `/etc/limeos` is root-owned and closed to others; an existing
  catalog is never replaced.
