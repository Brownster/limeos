# First wybie cutover attempt, 26 Nov 2026 (rolled back)

The first real migration of wybie (Raspberry Pi 5, Debian 12 arm64, systemd 252) from pi-health to
LimeOS, run by the operator per [docs/p06-cutover-runbook.md](../../../p06-cutover-runbook.md) as it
stood at `f657c24a`, with package `limeos 0.4.4` (arm64). `activate.sh` failed a health check and
rolled back; the frozen app was serving again within about a minute. Inputs copied from the host:
the preflight snapshot `/var/lib/limeos-cutover/20261126T010601Z` (small files in
[snapshot/](snapshot/units.txt); `ports.txt` without the tailscale listeners), `activate.log`, the
journal of the window and the host facts (`systemctl cat` of the old app's units, the container
list, the scan's problems), bundled under `raw/`.

Proven: from the files here. Reported: by the operator, not in these files.

## Timeline (UTC, from the journal and activate.log)

| Time | What |
| --- | --- |
| 01:04:17 | `caddy` installed from Debian; its package **starts Caddy** (default site, port 80). Stopped 01:04:19 by the operator. |
| 01:04:33 | `limeos` 0.4.4 installed: its configure set `/var/lib/limeos` to `root:root 0755` and started `limeos-storaged`, `limeos-containerd`, `limeos-core` ("core ready", generation 1). |
| 01:04:42 | core restarted (origin set in `core.json`). |
| 01:05:49 | containerd restarted (container ceiling installed). |
| 01:06:01 | `preflight.sh` passed (`--no-playback-check`; snapshot records the Rust services **active and enabled**). |
| 01:06:43 | `activate.sh`: frozen units stopped and disabled, `dpkg -i` (0.4.4 over 0.4.4), Rust services and the stack reader started, Caddy site installed and Caddy started on 8002. |
| 01:07:01-37 | Health checks: core health, login page, `limeosctl status`, managed containers, untouched units all passed; **stack reader ready failed** after 30 tries: `container_health` read `failed`, `the read failed: Unavailable` (a product bug, fixed separately). |
| 01:07:38-50 | Rollback (`return.sh`): Caddy stopped and disabled, the reader stopped, the Rust services **started again** (snapshot said active), then pi-health and its helper and timers started again. |
| 01:07:50 | systemd at pi-health's start: `ConfigurationDirectory 'limeos' already exists but the mode is different (755 vs 750)`; and, from `StateDirectory=limeos`, `/var/lib/limeos` chowned recursively to `holly:pihealth`, mode 0750. |
| 01:07:50 onward | The still-running core: `StateNotDurable` every 2 s ("background task failed", "superseding settled job resolution requests failed"). |
| 01:07:51 | `return.sh` verification: one failure, `live state file changed after the writers stopped: /var/lib/limeos/agent-actions/supervision.sqlite3`; `ROLLBACK INCOMPLETE`. The managed containers' IDs and StartedAt and the frozen port 8002 verified (no failure named for them). |
| 01:08:33-01:09:11 | The operator restarted the Rust services: core exited `StateNotDurable` ten times and hit its start limit; containerd looped on `unsafe or unavailable receipt store`. |
| 01:09:55 | After the operator's `chown -R` of `core/`, `assistant/`, `executors/*` back to their accounts and `chmod o+x /var/lib/limeos`: core ready (generation 4), for diagnosis. |
| 01:12:38 | Rust services stopped. |

## After the night

Reported: the host was verified identical to before the window: the fourteen managed containers with
the same IDs, StartedAt and restart counts, pi-health answering 200 on 8002, Jellyfin 200. Proven for
IDs, StartedAt and port 8002 by `return.sh`'s own verification above.

Reported state left on the host:

- `limeos` 0.4.4 installed, its units stopped and disabled.
- Caddy installed, stopped and disabled; `/etc/caddy/Caddyfile` is the import line, `/etc/caddy/conf.d` exists.
- `/etc/limeos/system-policy/container.json` with the fourteen containers and the stacks `mattermost` and `media`.
- `/etc/limeos/management-catalog.json` installed.
- One user imported into core's database.
- Snapshot `/var/lib/limeos-cutover/20261126T010601Z`, marked by the failed return (`activate.log`
  ends `ROLLBACK INCOMPLETE`). It cannot be activated again (a new activation needs a new preflight).

## Findings and what changed (branch `pr-cutover-shared-state`)

1. **The old app's units own the shared state directory.** `pi-health.service` and
   `limeos-metrics-collector.service` run as `holly:pihealth` with `StateDirectory=limeos`,
   `StateDirectoryMode=0750` ([host facts](EVIDENCE-BUNDLE.json "bundled: raw/host-facts.txt")).
   Each start sets the mode and, when the owner differs, chowns the directory and everything in it,
   LimeOS's trees included. Now: the package's configure leaves a directory another account owns as
   it is and does not start LimeOS; activation guards those units, makes the directory root's and
   repairs LimeOS's trees; `return.sh` gives it back as recorded before the old app starts and leaves
   LimeOS stopped and disabled, and otherwise verifies restarted Rust services still work.
2. **False `ROLLBACK INCOMPLETE`.** `supervision.sqlite3` is written by the `limeops-*` services, which
   keep running; `auto_update.json` and `backup_config.json` are rewritten by schedules outside the
   frozen app. Now: a live list classifies such state; changes are reported, never failed on.
3. **`migrate scan` incomplete.** The agent supervisor's virtualenv (1,096 files, 74 directories past
   depth 8) hit the bound; the state list had 1,093 venv files of 1,237 lines. Now: a virtualenv is one
   entry, not descended; `state_lists.py` builds reviewable lists.
4. **Layout and runbook.** The loose scripts could not find the Caddy site template, and preflight did
   not check it. Now: `bundle.sh` builds the directory to copy (with `SHA256SUMS`), `activate.sh` finds
   the template next to itself (`--site-template`), preflight checks every activation input; the
   runbook's step 1 records the package, ceilings, catalog, Caddy and users steps as performed.

## Files

`snapshot/` (tracked): the snapshot's small records. `raw/` (bundled): `activate.log`, `journal.log`,
`host-facts.txt`, `state-paths.txt.gz`, `state.sha256.gz`, `state.poststop.sha256.gz`. No file holds a
secret: the snapshot records paths, hashes and unit and container names only.
