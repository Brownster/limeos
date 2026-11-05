# Basic cutover tooling: scope

Status: scoping, 5 November. Covers only the agreed basic cutover (host metrics,
container status/logs/approved start-stop-restart, stack status and whole-stack
start/stop/restart). It does not close full P06; the operator's waiver of the
remaining gates (storage mounts, backup/restore, schedules, assistant) applies.
Requirements: RW-060 to RW-062 and MIG-001 to MIG-004 in the
[roadmap](2026-10-04-rust-rewrite-roadmap.md) and
[defect register](2026-10-04-rust-rewrite-defect-register.md).

## What exists

- `Store::import_legacy_user` (crates/persistence/src/identity_store.rs) validates
  Werkzeug scrypt/PBKDF2 hashes and creates an administrator. Only tests call it.
- `limeosctl` has `status`, `bootstrap`, `enroll`, `check-ceiling` only.
- The frozen app keeps users in an environment file (`PIHEALTH_USERS=name:hash,...`
  or `PIHEALTH_USER` + `PIHEALTH_PASSWORD_HASH`), not a database.
- Per-action ceilings (`allow_*`, `managed_containers`, new `allow_stack`,
  `managed_stacks`) default to off, so a fresh install can read but not act.

## What the basic cutover needs

Everything below is read-only against the frozen app. Nothing edits its files.

| Id | Tool | Behaviour | Done when |
|---|---|---|---|
| T1 | `limeosctl migrate scan` | Root-only, read-only. Lists every state source (env/credentials file, `/etc/limeos`, `/var/lib/limeos` SQLite and JSON, stacks root, checkout edits) with size/owner/hash. Marks each imported, deferred or unsupported. Redacted report (no hashes, tokens, paths of secrets' contents). | Report names every source; original file hashes identical before and after. |
| T2 | `limeosctl migrate import-users` | Root-only. Reads hashes from the env file and calls a new root-peer `CoreRequest::ImportLegacyUsers` (same trust model as `enroll`; never argv, never plaintext). Idempotent, atomic, all-or-nothing. | A hash made by the frozen app logs in; hash upgrades to Argon2id after login; second run changes nothing; bad entry imports nothing. |
| T3 | `limeosctl migrate propose-ceiling` | Reads `docker ps` + stack catalog, writes a proposed `system-policy` file with every `allow_*` false and the exact container IDs/names in `managed_*`. Operator reviews and enables. | `check-ceiling` accepts it; nothing acts until flags are set. |
| T4 | `scripts/cutover/preflight.sh` | Snapshot before anything changes: unit/timer states, listening ports, `docker ps` IDs, original-state hashes, free memory/swap, 16 KiB page check, HTTPS proxy presence. Refuses to continue under memory pressure or active playback. | Snapshot file written; refuses on a failed check. |
| T5 | `scripts/cutover/activate.sh` | Stops and disables the frozen writers (identified in T1: app, helper, schedulers, alert daemon, update/backup timers), installs the pinned package, starts the new services behind the HTTPS proxy on the agreed port, health-checks, and rolls back automatically on any failed check. | Rehearsed on a VM and then the Pi; containers' IDs and uptime unchanged. |
| T6 | `scripts/cutover/return.sh` | Stops the new services, restores the frozen units and port, re-verifies original-state hashes against the T4 snapshot and container IDs against `docker ps`. | Rehearsed after both a clean and a failed activation; hashes and containers match. |

## Deliberately out of scope for the basic cutover

Importing the SQLite/JSON history, alert policies, schedules, backup settings and
agent state (reported as deferred by T1; the frozen app's data stays in place for
the return path); ownership migration (MIG-003): stack directories stay owned by
`holly`, and `holly` stays in the `docker` group, with no group change without the
operator's explicit say-so; storage mounts; backups; the assistant.

## Read-only findings from wybie (5 November, 20:40 GMT)

Collected with read-only commands; secret values were masked and nothing changed.

- Hardware: 16 KiB pages, kernel 6.12.25+rpt-rpi-2712, idle, 5.9 GB available,
  swap 401/511 MB used (already under pressure; the new services are about 10 MiB).
- The frozen app is `pi-health.service` running `app.py` as `holly` from
  `/home/holly/pi-health` (HEAD `618ce92`; edited `config/backup_config.json` and
  `config/media_paths.json`; untracked `.venv`, `copyparty`, `config/storage_plugins`).
  It listens on plain HTTP `0.0.0.0:8002`. **No HTTPS proxy is installed** (caddy,
  nginx, traefik inactive), but the Rust UI needs HTTPS, so the cutover must also
  install and configure a proxy (`packaging/Caddyfile.example`) and decide the port.
- Users: `/etc/limeos/credentials.env` (`holly:pihealth`, 0640) has one
  `PIHEALTH_USERS` line plus `DISK_PATH`, `DISK_PATH_2`. `T2` reads exactly this.
- Frozen writers to stop at cutover: `pi-health`, `pihealth-helper` (root),
  `limeos-metrics-collector.timer` (5 min), `limeos-package-reconcile.timer`,
  plus app-driven `backup_config.json` / `auto_update.json`.
- Other services that are not ours to stop: `flask-shutdown` (root),
  `limeops-*` (action worker, actuatord, report scheduler, supervised repair,
  read-only broker), `limeos-agent` (Mattermost assistant). Their dependence on
  `pi-health` is unknown and must be checked before anything is stopped.
- State: five SQLite files under `/var/lib/limeos` (`agent-actions/{actions,automation,findings,supervision}`,
  `metrics`) and JSON for auto-update, backup, integrations, package approvals and
  pending actions; `/etc/limeos` also holds about 50 timestamped policy backups.
  T1 must report all of these and import none of them.
- Docker: `holly`, `limeops`, `limeops-actuator` are in the `docker` group.
  14 containers run (jellyfin, radarr, sonarr, sabnzbd, transmission, jackett, vpn,
  navidrome, audiobookshelf, rdtclient, requestrr, get_iplayer, yt-to-jellyfin,
  watchtower). Stopped: airsonic-advanced and the Mattermost stack's three
  (`limeos-mattermost*`, `limeos-alertd`). Stacks root is `/opt/stacks` (`media`, `mattermost`), owned by `holly`.
- Port 8009 is `docker-proxy`; 8002 is the only listener a new build would replace.

Dependency check (read-only, same evening): no `limeops-*`, `limeos-agent` or
`flask-shutdown` unit requires `pi-health`; `limeopsd` is only ordered after
`pihealth-helper`, and nothing under `/usr/lib/limeos-agent` calls port 8002. The
only link is `agent-policy.json`, which allow-lists the unit names `pi-health` and
`pihealth-helper` for the agent to inspect. `pihealth-helper` is `PartOf=pi-health`,
so stopping `pi-health` stops the helper too. No client was connected to 8002.
Stopping the frozen app therefore does not break the Mattermost/limeops services; the
agent will just see those named units as inactive.

Still unknown: whether the new stack reader can read `/opt/stacks` as its service user; which port and TLS
setup the household's browsers use today.

## Order of work

1. T1 + T2 (one engineer; mostly local, the user import is testable without a host).
2. T3 (small).
3. Read-only Pi inspection in the first window, answering the questions above.
4. T4-T6 against the answers; rehearse on a disposable VM with a redacted copy of the
   state first, then the Pi in a later approved window, return path included.

Cutover itself needs the operator's explicit go on a fresh window. No step in this
plan activates anything on the Pi.
