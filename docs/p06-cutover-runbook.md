# P06 basic cutover runbook

Scope and findings: [tooling scope](plans/2026-11-05-basic-cutover-tooling-scope.md).
The scripts live in `scripts/cutover/`. Nothing here runs without the operator's
go on a fresh window; the scripts were tested against stubs and then rehearsed end to end in a disposable VM
([evidence](rewrite-evidence/p06/2026-11-06-cutover-rehearsal-vm/README.md)).

## Open decisions (operator)

1. **TLS mode - OPEN.** `internal` (default): Caddy's local CA signs the cert, so
   no outside dependency, but every household browser must trust that CA once or
   it shows a warning, and the UI needs a secure context. Own certificate
   (`--tls-cert/--tls-key`): no per-browser step if the CA is already trusted, but
   the operator must supply, renew and protect the key.
2. **Public port - OPEN.** Default 8002 so bookmarks survive (Caddy takes the port
   only after the frozen app stops, so a return must stop Caddy first; the
   scripts do). Any other port means new bookmarks and a different `origin`.
3. `core.json` `origin` must equal `https://HOST:PORT` exactly (edit before the
   window; `activate.sh` refuses otherwise).
4. Caddy is not installed on the host and the scripts do not install it. Install
   it first and put `import /etc/caddy/conf.d/*.caddy` in `/etc/caddy/Caddyfile`.
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
   It stops and disables only `limeos-metrics-collector.timer`,
   `limeos-package-reconcile.timer`, `pi-health` and `pihealth-helper`; installs
   the package; starts the Rust services and the read-only stack reader
   (`limeos-storage-reader`, started but never enabled); installs the Caddy site; then checks
   core health and the login page through the proxy, `limeosctl status`, `docker
   ps` IDs and StartedAt against the snapshot, that the reader is active, listening and not
   restarting (twice, three seconds apart), and that `limeops-*`,
   `limeos-agent` and `flask-shutdown` are unchanged. Any failure runs
   `return.sh` and exits non-zero.
5. Log in through the browser (trust the Caddy CA first under `internal`).
6. `scripts/cutover/return.sh SNAP [--dry-run]` at any time: removes the Caddy
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
