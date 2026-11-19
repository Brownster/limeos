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
   `preflight.sh --managed-file`). Only these must keep their IDs and StartedAt
   through the cutover; a restart of any other container is reported by name and
   does not roll back. `watchtower` is in the default list, so with the defaults a
   watchtower restart, or an update it applies to a managed container, rolls back.

## Steps

1. Read-only checks: `limeosctl migrate scan`, then `import-users` and
   `propose-ceiling` per [migrate tools](p06-migrate-tools.md). Install the
   ceiling and the `.deb` set; choose the pinned version. Generate and review the
   stack catalog: `limeosctl migrate propose-catalog --out
   /root/management-catalog.proposed.json`, read its `stack ...` / `refused ...` lines (a
   Compose project outside `/opt/stacks`, such as wybie's one-container `docker`, is refused by
   design), then `install -o root -g root -m 0644 /root/management-catalog.proposed.json
   /etc/limeos/management-catalog.json` (or rerun with `--install`).
   Build the state list (the frozen app's files that must not change) from the
   scan report, then read it and drop anything that is not the frozen app's:

   ```sh
   limeosctl migrate scan --json --checkout /home/holly/pi-health/config >/root/migrate-scan.json
   python3 -c 'import json,sys; r=json.load(open(sys.argv[1])); loc={l["label"]: l["path"].rstrip("/") for l in r["locations"]}; print("\n".join(loc[f["location"]] + "/" + f["path"] for f in r["files"] if f["kind"] == "file" and f["location"] != "stacks"))' /root/migrate-scan.json >/root/cutover-state-paths.txt
   ```

   Create a Jellyfin API key (Dashboard > API Keys) and store it root-only:
   `install -o root -g root -m 0600 /dev/null /root/jellyfin.key`, then write the
   key into that file with an editor (not on a command line).
2. `scripts/cutover/preflight.sh --state-list /root/cutover-state-paths.txt
   --jellyfin-key-file /root/jellyfin.key`
   (writes `/var/lib/limeos-cutover/<ts>/`, or `--out DIR`). It refuses on
   MemAvailable below 1.5 GiB, swap above 90 %, a stopped managed container,
   Jellyfin playback, a listed state path that does not exist, a state path that
   is not a regular file (directories and symlinks included) or cannot be hashed,
   no state found at all, or a missing/unsafe
   `/etc/limeos/management-catalog.json` (the message names the command above).
   The key is sent to Jellyfin's `/Sessions` on curl's stdin, never on a command
   line or into the snapshot. It passes only on a complete HTTP 200 whose body
   is a JSON list of sessions with none playing (parsed with `python3`); a key
   file that is a symlink or that group or others can read, a rejected key, an
   unreachable Jellyfin, a transfer cut short or any other body refuses. Without a key file it
   refuses unless you pass `--no-playback-check`; then check the dashboard
   yourself.
3. Rehearse: `scripts/cutover/activate.sh SNAP --dry-run --package PATH
   --expected-version V --host HOST` prints every command and changes nothing;
   interrupting it never starts a rollback.
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
   ps` IDs and StartedAt of the managed containers against the snapshot (naming
   any that changed; other containers that changed are warned about by name), that the reader is active, listening and not
   restarting (twice, three seconds apart), and that `limeops-*`,
   `limeos-agent` and `flask-shutdown` are unchanged. Any failure runs
   `return.sh` and exits non-zero; so do SIGINT, SIGTERM, SIGHUP and any
   unexpected exit after the first change. Each step runs in its own session, so
   a Ctrl-C lets the running step (dpkg included) finish before the rollback
   starts. The rollback runs once and ignores further signals until it ends.
   It runs detached: after the snapshot check it starts itself again in a new
   session (`systemd-run --scope`, then `setsid`), appends its output to
   `SNAP/activate.log` and the terminal only follows that log, so a dropped SSH
   session does not stop it. `systemd-run` is required; if it is missing or
   fails, nothing has changed. Reconnect and read the log, or `tail -f` it.
   Signals: Ctrl-C in the following terminal rolls the run back (passed on as
   SIGTERM); SIGTERM or SIGHUP to the follower only stops following and the run
   carries on; the run itself rolls back on SIGINT, SIGTERM or SIGHUP, so with
   the terminal gone, `kill -TERM` the pid it printed at the start.
   `--attached` keeps it in the foreground (inside tmux or screen, or for tests).
   One run per snapshot: while a run holds `SNAP/activate.lock` a second
   `activate.sh` on that snapshot is refused. After a completed activation the
   snapshot is marked `SNAP/activated`; rerunning on it, or activating while the
   cutover Caddy site exists, is refused before any change. After `return.sh`,
   a new activation needs a new preflight.
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
   site, returns the Rust services to their snapshot state (stopped and disabled
   unless they were already running or enabled before the cutover), re-enables
   and starts the frozen units (helper, app, timers), then verifies state hashes,
   the managed containers' IDs and StartedAt (other containers only warn), and
   that port 8002 answers over plain HTTP. It hashes the state files before the
   frozen units restart but compares them afterwards, so a hashing failure is
   reported and never leaves the frozen app stopped. Safe to repeat. A snapshot
   from before state lists and managed sets (no `state-paths.txt` or
   `managed.txt`) is still accepted, with a warning: the paths in its
   `state.sha256` are compared and every container must match; `activate.sh`
   refuses such a snapshot. The
   package stays installed.

Stack status and whole-stack start/stop/restart need the reader and the catalog; `activate.sh`
starts the reader (it has no Install section, so it never starts at boot) and `return.sh` stops
it. Log in and open the stacks page to confirm it is no longer `unavailable`. After a reboot the
reader is not running: start it by hand (`systemctl start limeos-storage-reader`). Re-run `propose-catalog` and replace the catalog (as root, by hand) whenever a
Compose file under `/opt/stacks` changes, since its hash is registered.

## Abort criteria

Run `return.sh` and stop if any of these holds: a preflight refusal you cannot
explain; `activate.sh` rolled back (read its output, do not retry blind); a
managed container's ID or StartedAt changed; a state hash changed; swap or memory
pressure rises during the window; login or health fails after activation; the
household reports playback trouble. If `return.sh` exits non-zero, do not
retry the cutover: inspect the named failure by hand.

## Notes

- Hashes cover every path in the state list (copied into the snapshot as
  `state-paths.txt`, which `activate.sh` and `return.sh` reuse) and
  everything under `/etc/limeos` and `/var/lib/limeos` except the new package's
  paths. On wybie the list must include the two edited `pi-health/config` files.
  Paths may hold any byte except a newline; a file whose name has one refuses
  preflight. SQLite files and journals can change while the writers run, so they are
  checked against a baseline taken just after the writers stop (`return.sh`
  warns only if no baseline exists).
- Snapshot contents are paths, hashes and unit/container names, never file contents.
