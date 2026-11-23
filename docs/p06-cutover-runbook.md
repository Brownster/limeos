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
   no state found at all, a missing tool (each named: `flock`, `setsid`,
   `systemd-run`, an `env` with `--default-signal` from coreutils 8.31 or newer,
   and `python3` unless `--no-playback-check`; `activate.sh` checks the same
   before any change), or a missing/unsafe
   `/etc/limeos/management-catalog.json` (the message names the command above).
   The key is sent to Jellyfin's `/Sessions` on curl's stdin, never on a command
   line or into the snapshot. It passes only on a complete HTTP 200 whose body
   is a JSON list of sessions with none playing (parsed with `python3`); a key
   file that is a symlink or that group or others can read, a rejected key, an
   unreachable Jellyfin, a transfer cut short or any other body refuses. Without a key file it
   refuses unless you pass `--no-playback-check`; then check the dashboard
   yourself.
   Every query preflight records or gates on has three answers: yes, no, and
   unknown (it timed out after 30 s or gave no answer). Unknown is a refusal
   naming the query, never a guess: `could not determine whether UNIT is active`
   (or `enabled`), `systemd did not list its unit files` (the `limeops-*` units),
   `could not list the running containers`, `could not determine whether the
   limeos package is installed`, `could not list the listening TCP ports`. So
   `SNAP/units.txt` always records a definite state for every unit; a unit file
   that does not exist is recorded as `not-found` (when `is-enabled` prints
   nothing for it, as older systemd versions do, preflight asks for its
   `LoadState`; to confirm on the host which form systemd 252 uses). `activate.sh` and
   `return.sh` refuse, before any change, a snapshot whose `units.txt` has a
   blank, unknown or missing state, as they refuse a missing hash.
   Preflight also checks that the cgroup2 unified hierarchy with `cgroup.kill`
   (kernel 5.14 or newer) is available, which `return.sh --force` needs. If not,
   it passes with `WARN: the cgroup2 unified hierarchy with cgroup.kill is not
   available (REASON): return.sh --force could not end a hung activation by its
   scope and would stop without killing anything, unless run with
   --allow-systemctl-kill-fallback (...)`; decide before the window whether you
   accept that (see step 7).
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
   starts. That wait is bounded: after 5 minutes (the longest legitimate step,
   `dpkg -i`, restarts three services within systemd's 90 s start timeouts) the
   step's session gets SIGTERM, then SIGKILL 10 s later, and the rollback goes
   ahead with a TIMEOUT line naming the step. If even SIGKILL has not ended it
   after 5 s (a dpkg stuck in uninterruptible I/O), the rollback starts anyway:
   the TIMEOUT line names its pid and `SNAP/stuck-step` records it. `return.sh`
   then fails with that pid if it is still alive, and a later `return.sh` refuses
   while it is (see step 7). Wait for it to exit
   (`ps -o pid,stat,args -p PID`), run `dpkg --audit`, then `dpkg --configure -a`
   or `dpkg --remove limeos` as it reports, and run `return.sh SNAP` again. The rollback runs once and ignores further signals until it ends.
   With the old service down, the rollback never gives up on `SNAP/return.lock`:
   if another `return.sh` holds it, the rollback waits until it is released and
   prints every 30 s `ROLLBACK: waiting for SNAP/return.lock, held by return.sh
   (pid P, state S) since T`. That holder either finishes the return or refuses
   (seeing the activation's lock) and releases it, and the kernel releases it if
   the holder dies. If the state is `T` (a `return.sh` suspended with Ctrl-Z),
   resume it with `kill -CONT P`. Every query the scripts make (`systemctl`,
   `docker`, `dpkg-query`, `ss`, `limeosctl status`) gives up after 30 s with a
   `timed out after 30s: ...` warning naming it; one that answers with nothing
   is named in a `no answer from: ...` warning. Each health check is tried 30
   times; if the last try could not get an answer, the check fails with
   `health check failed: LABEL: could not determine after 30 tries (DETAIL)`
   followed by that try's warnings, otherwise with `...: was false after 30
   tries (DETAIL)`; either way the activation rolls back. An unreadable restart
   count of the stack reader is such a failure, never a zero. If systemd cannot
   say whether Caddy runs when the site is installed, the step is `systemctl
   reload-or-restart caddy.service`, which loads the site either way.
   The step is signalled by its process group, which a helper process in that
   group keeps from being reused until the script has finished with the step.
   Every activation runs inside a transient systemd scope named for the
   snapshot, `limeos-cutover-activate-<first 32 hex digits of the SHA-256 of the
   snapshot's real path>.scope`, so two snapshots never share one (printed at the
   start, recorded in `SNAP/activation-scope` and `SNAP/activate.lock`); everything it starts, steps and
   rollback included, stays in that scope. `systemd-run` is required, with
   `--attached` too; if it is missing or cannot create the scope, nothing has
   changed.
   It runs detached: after the snapshot check it starts itself again in that
   scope and a new session (`setsid`), appends its output to
   `SNAP/activate.log` and the terminal only follows that log, so a dropped SSH
   session does not stop it. Reconnect and read the log, or `tail -f` it.
   Signals: Ctrl-C in the following terminal rolls the run back (passed on as
   SIGTERM); SIGTERM or SIGHUP to the follower only stops following and the run
   carries on; the run itself rolls back on SIGINT, SIGTERM or SIGHUP, so with
   the terminal gone, `kill -TERM` the pid it printed at the start.
   `--attached` keeps it in the foreground (inside tmux or screen, or for tests):
   the same process enters the scope and keeps the terminal.
   One run per snapshot: while a run holds `SNAP/activate.lock` a second
   `activate.sh` on that snapshot is refused. After a completed activation the
   snapshot is marked `SNAP/activated`; rerunning on it, activating while the
   cutover Caddy site exists, or activating while `SNAP/force-takeover` records an
   unfinished `return.sh --force`, is refused before any change. After `return.sh`,
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
   targets, each as pass, fail, unmeasured or unknown. A row is unknown (no
   value, a note saying what could not be determined, and `unknown_units` in the
   report) when `systemctl show` did not answer for a unit's processes or
   `dpkg-query` for the package; it is never reported as a number or a pass:
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

   Exit 0 means every measured target passed; exit 2 means a target failed, a
   LimeOS process was missing or a row is unknown (the report is still written). A miss never runs
   `return.sh`: read the report and decide yourself whether it is an abort
   criterion. Keep `report.json` with the cutover evidence.
6. Log in through the browser (trust the Caddy CA first under `internal`).
7. `scripts/cutover/return.sh SNAP [--dry-run] [--force]` at any time, except
   while an activation on that snapshot is still running. Only one `return.sh`
   runs per snapshot: each takes `SNAP/return.lock` first and holds it to the end,
   so a second one is refused and names the first. Then it takes
   `SNAP/activate.lock`; while an activation holds that, it refuses at once,
   before any systemd query, and names the
   activation's pid and scope (each script records its role, pid and start time
   in the locks; a record whose process has gone is reported as stale, never as a
   pid to signal). That run rolls back by itself if it fails; `kill -TERM PID`
   makes it roll back now. It also refuses while the activation's scope still
   has processes after `activate.sh` itself has gone (a step it left behind), and
   lists them. `--force` does not end those: with `activate.lock` free there is no
   live activation to vouch for the unit, so it kills nothing and names the
   command to run yourself once `systemd-cgls --unit UNIT` shows they are the
   activation's (`systemctl kill --kill-whom=all --signal=KILL UNIT`).

   `--force` is only for an activation that is hung. Before killing anything it
   checks that `SNAP/activate.lock` is held by a current `activate.sh` record (pid
   and start time) that names the scope in `SNAP/activation-scope`, and that this
   process is inside that scope's control group; otherwise it stops with
   "nothing was killed or changed". It ends the activation by its
   scope, never by pid: it writes `1` to the scope's
   `/sys/fs/cgroup/system.slice/UNIT/cgroup.kill` (cgroup v2, kernel 5.14 or
   newer), so the kernel kills `activate.sh`, its automatic rollback and every
   step at once, including any forked during the kill, and nothing outside the
   scope (a backup started from the same shell is untouched). If that file
   cannot be used (not a cgroup2 mount, an older kernel, a failed write) it kills
   nothing: it prints `ERROR: --force: cannot end the activation's scope UNIT
   through the kernel's cgroup.kill: REASON. Nothing was killed or changed.`,
   then `Inspect: systemctl status UNIT; systemd-cgls --unit UNIT`, the command
   to end it yourself once those are the activation's processes (`systemctl kill
   --kill-whom=all --signal=KILL UNIT`), and `return.sh SNAP` to run once
   `systemctl is-active UNIT` prints `inactive`. On such a host
   `return.sh SNAP --force --allow-systemctl-kill-fallback` lets `return.sh` run
   that `systemctl kill` itself. Caveat: on systemd 252 systemd lists the scope's
   pids and signals each one, so a process that exits in between can have its pid
   reused by an unrelated process, which would then be killed; check the scope
   with `systemd-cgls` first. The output says which kill it used and why. It then
   waits up to 15 s for the scope to empty
   and only then takes `activate.lock` and returns, holding `return.lock` the
   whole time. If the activation was inside dpkg, run `dpkg --audit` afterwards.
   Before the kill it writes `SNAP/force-takeover` and removes it once the scope
   is empty.

   If the scope does not empty (a process in uninterruptible I/O, such as dpkg
   waiting on a failing disk, cannot exit until that I/O ends), `--force` stops
   having changed nothing else and holding no lock, and prints one message: the
   unit, each process still in it (pid, state, command; one that exits while the
   list is made is shown as such), that the activation's automatic rollback was
   ended and the old service is still stopped, and the next step: wait until
   `systemctl is-active UNIT` prints `inactive`, then run `return.sh SNAP --force`
   again. Until then a plain `return.sh` explains that an earlier forced return
   did not finish (instead of reporting a stale lock), and a new activation is
   refused. Once the scope is empty, a plain `return.sh` completes the return too.
   To inspect the unit:

   ```sh
   systemctl status limeos-cutover-activate-...scope     # name from the message or SNAP/activation-scope
   systemd-cgls --unit limeos-cutover-activate-...scope
   cat /sys/fs/cgroup/system.slice/limeos-cutover-activate-...scope/cgroup.procs
   ps -o pid,stat,wchan:32,args -p PID                   # state D: waiting on I/O
   ```

   Otherwise: removes the Caddy
   site, returns the Rust services to their snapshot state (stopped and disabled
   unless they were already running or enabled before the cutover), re-enables
   and starts the frozen units (helper, app, timers), then verifies state hashes,
   the managed containers' IDs and StartedAt (other containers only warn), and
   that port 8002 answers over plain HTTP. It hashes the state files before the
   frozen units restart but compares them afterwards, so a hashing failure is
   reported and never leaves the frozen app stopped; each path is verified on
   its own, so one that cannot be hashed fails alone and the rest are still
   checked. Safe to repeat. A snapshot
   from before state lists and managed sets (no `state-paths.txt` or
   `managed.txt`) is still accepted, with a warning: the paths in its
   `state.sha256` are compared and every container must match; `activate.sh`
   refuses such a snapshot. The
   package stays installed.

   Frozen units always follow the snapshot. When systemd cannot say what state
   a unit is in now, `return.sh` takes the restoring action anyway and says the
   state was unknown: `could not determine whether caddy.service is running
   ...; reloading it anyway` (`systemctl try-reload-or-restart caddy.service`,
   which reloads a running Caddy and leaves a stopped one alone) and `could not
   determine whether limeos-storage-reader.service is running ...; stopping it
   anyway`. Each corrective action gives up after 180 s (twice systemd's default
   start/stop timeout); one that does not finish is reported as `timed out after
   180s: CMD; this step of the return is incomplete ...; check it, then run by
   hand: CMD`, the remaining steps still run, and `return.sh` ends with `return
   incomplete: N failure(s)` (exit 1; under `activate.sh` this prints `ROLLBACK
   INCOMPLETE`). During verification a query that cannot answer is a failure that
   says so (`could not determine the containers' IDs and StartedAt`, `could not
   determine the state of untouched unit UNIT`), and a live database that differs
   from the baseline is not excused when whether the frozen writers were running
   could not be determined.

   Any of the four scripts that stops on a failure it did not expect names it:
   `ERROR: return.sh stopped unexpectedly (exit 1): 'COMMAND' failed with status
   N at FILE:LINE in FUNCTION, called from ...`; `activate.sh` then rolls back
   whatever it had changed.
   Keep that line for the fix; `return.sh` is safe to run again.

   A snapshot from an older preflight may hold a blank unit state; `return.sh`
   refuses it before any change, naming the record. Write the state the unit had
   before the cutover into `SNAP/units.txt` (for example `managed
   pi-health.service active enabled`) and run it again, or restore by hand.

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
