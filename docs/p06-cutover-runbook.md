# P06 basic cutover runbook

Scope and findings: [tooling scope](plans/2026-11-05-basic-cutover-tooling-scope.md).
The scripts live in `scripts/cutover/`. Nothing here runs without the operator's
go on a fresh window; the scripts were tested against stubs and then rehearsed end to end in a disposable VM
([evidence](rewrite-evidence/p06/2026-11-06-cutover-rehearsal-vm/README.md)).

## Operator decisions

1. **TLS mode - DECIDED 2026-11-12: `internal`.** Caddy's local CA signs the cert, so
   no outside dependency, but every household browser must trust that CA once or
   it shows a warning, and the UI needs a secure context. (Not chosen: own certificate
   via `--tls-cert/--tls-key`.)
2. **Public port - DECIDED 2026-11-12: 8002**, so bookmarks survive with an `https://`
   scheme (Caddy takes the port only after the frozen app stops, so a return must
   stop Caddy first; the scripts do).
3. `core.json` `origin` must equal `https://HOST:8002` exactly (edit before the
   window; `activate.sh` refuses otherwise). HOST is the name household browsers
   use; assumed `wybie`, to confirm.
4. **Caddy install - APPROVED 2026-11-12.** Caddy is not installed on the host and
   the scripts do not install it. Install it in the cutover window before
   `activate.sh` and put `import /etc/caddy/conf.d/*.caddy` in `/etc/caddy/Caddyfile`.
   Caddy may also bind port 80 for redirects; if that port is taken, set
   `auto_https disable_redirects` in the main Caddyfile's global block.
5. Whether the managed-container list is the default fourteen names (edit with
   `preflight.sh --managed-file`).

## Steps

1. Read-only checks: `limeosctl migrate scan`, then `import-users` and
   `propose-ceiling` per [migrate tools](p06-migrate-tools.md). Install the
   ceiling and the `.deb` set; choose the pinned version. Generate and review the
   stack catalog: `limeosctl migrate propose-catalog --out
   /root/management-catalog.proposed.json`, read its `stack ...` / `refused ...` lines (a
   Compose project outside `/opt/stacks`, such as wybie's one-container `docker`, is refused by
   design), then `install -o root -g root -m 0644 /root/management-catalog.proposed.json
   /etc/limeos/management-catalog.json` (or rerun with `--install`).
2. `scripts/cutover/preflight.sh` (writes `/var/lib/limeos-cutover/<ts>/`, or
   `--out DIR`). It refuses on MemAvailable below 1.5 GiB, swap above 90 %, a
   stopped managed container, Jellyfin playback, or a missing/unsafe
   `/etc/limeos/management-catalog.json` (the message names the command above). Without Jellyfin credentials
   it cannot see playback and warns: check the dashboard yourself.
3. Rehearse: `scripts/cutover/activate.sh SNAP --dry-run --package PATH
   --expected-version V --host HOST` prints every command and changes nothing.
4. Go (within 30 minutes of step 2, or rerun preflight):
   `scripts/cutover/activate.sh SNAP --yes-i-have-operator-approval --package PATH
   --expected-version V --host HOST [--port 8002] [--tls-cert F --tls-key F]`.
   `--package` names the one `limeos_V_ARCH.deb` file (not a directory); its
   `Package` and `Version` fields must be `limeos` and `V`, checked before
   anything changes.
   It stops and disables only `limeos-metrics-collector.timer`,
   `limeos-package-reconcile.timer`, `pi-health` and `pihealth-helper`; installs
   that file alone with `DEBIAN_FRONTEND=noninteractive dpkg --force-confold -i`,
   so dpkg never prompts and keeps your edited `/etc/limeos/core.json` (a
   conffile); starts the Rust services and the read-only stack reader
   (`limeos-storage-reader`, started but never enabled); installs the Caddy site; then checks
   core health and the login page through the proxy, `limeosctl status`, `docker
   ps` IDs and StartedAt against the snapshot, that the reader is active, listening and not
   restarting (twice, three seconds apart), and that `limeops-*`,
   `limeos-agent` and `flask-shutdown` are unchanged. Any failure runs
   `return.sh` and exits non-zero; so do SIGINT, SIGTERM, SIGHUP and any
   unexpected exit after the first change. The rollback runs once and ignores
   further signals until it ends.
   It runs detached: after the snapshot check it starts itself again in a new
   session (`setsid`, inside `systemd-run --scope`), appends its output to
   `SNAP/activate.log` and the terminal only follows that log, so a dropped SSH
   session does not stop it. Reconnect and read the log, or `tail -f` it. Ctrl-C
   in the following terminal rolls the run back (it is passed on as SIGTERM);
   with the terminal gone, `kill -TERM` the detached `activate.sh` to roll back.
   `--attached` keeps it in the foreground (inside tmux or screen, or for tests).
5. Measure, once `activate.sh` has passed its health checks and the host is idle:
   `scripts/cutover/measure.sh --host HOST [--port 8002] [--out DIR] [--window 600]`.
   It is read-only: it reads `/proc`, `/sys/fs/cgroup`, `systemctl show` and
   `dpkg-query`, sends loopback HTTPS requests through the proxy, and writes only
   `DIR/report.json` and `DIR/raw.txt` (default
   `/var/lib/limeos-cutover/measure-<ts>/`). It never calls docker and never reads
   `/opt/stacks` or the frozen app's files. Over the idle window (`--window`,
   default 600 s) it records the
   [architecture section 10](plans/2026-10-04-rust-rewrite-architecture.md#10-observations-performance-and-footprint)
   targets, each as pass, fail or unmeasured:
   - PSS and swap (`/proc/PID/smaps_rollup`) of every process in the
     `limeos-core`, `limeos-storaged` and `limeos-containerd` cgroups: at most
     30 MiB PSS combined, no swap; with any running `limeos-storage-*` executor,
     at most 60 MiB PSS for all resident LimeOS processes.
   - CPU (`/proc/PID/stat`, including reaped children) over the window: under
     0.1 % of one core.
   - Disk writes (`/proc/PID/io` `write_bytes`) over the window, extrapolated:
     under 20 MB/day.
   - p95 of `/api/v1/health` through Caddy on 127.0.0.1 (`--requests`, default
     100), minus TCP/TLS setup: under 20 ms. The authenticated cached read
     (`/api/v1/overview`) needs a session that `limeosctl` cannot provide, so it is
     unmeasured unless you pass `--cookie-file FILE`, a curl cookie file with a
     browser session. The script reads the file and never copies it; delete it
     afterwards.
   - The `limeos` package footprint (the larger of dpkg `Installed-Size` and its
     installed files): under 154 MB.
   - Core readiness after a restart is skipped by default. `--measure-core-restart`
     restarts `limeos-core` to time it, which is the only change the script can
     make. Use it only with the household's agreement.

   Exit 0 means every measured target passed; exit 2 means a target failed or a
   LimeOS process was missing (the report is still written). A miss never runs
   `return.sh`: read the report and decide yourself whether it is an abort
   criterion. Keep `report.json` with the cutover evidence.
6. Log in through the browser (trust the Caddy CA first under `internal`).
7. `scripts/cutover/return.sh SNAP [--dry-run]` at any time: removes the Caddy
   site, stops and disables the Rust services, re-enables and starts the frozen
   units (helper, app, timers), then verifies state hashes, container IDs and
   StartedAt, and that port 8002 answers over plain HTTP. Safe to repeat. The
   package stays installed.

Stack status and whole-stack start/stop/restart need the reader and the catalog; `activate.sh`
starts the reader (it has no Install section, so it never starts at boot) and `return.sh` stops
it. Log in and open the stacks page to confirm it is no longer `unavailable`. After a reboot the
reader is not running: start it by hand (`systemctl start limeos-storage-reader`). Re-run `propose-catalog` and replace the catalog (as root, by hand) whenever a
Compose file under `/opt/stacks` changes, since its hash is registered.

## Abort criteria

Run `return.sh` and stop if any of these holds: a preflight refusal you cannot
explain; `activate.sh` rolled back (read its output, do not retry blind); a
container ID or StartedAt changed; a state hash changed; swap or memory
pressure rises during the window; login or health fails after activation; the
household reports playback trouble. If `return.sh` exits non-zero, do not
retry the cutover: inspect the named failure by hand.

## Notes

- Hashes cover `credentials.env`, the two edited `pi-health/config` files and
  everything under `/etc/limeos` and `/var/lib/limeos` except the new package's
  paths. SQLite files and journals can change while the writers run, so they are
  checked against a baseline taken just after the writers stop (`return.sh`
  warns only if no baseline exists).
- Snapshot contents are paths, hashes and unit/container names, never file contents.
