# Basic-cutover rehearsal in a disposable VM that imitates wybie

One disposable Debian 12 AMD64 KVM guest at a time, 4 GiB RAM (`tests/privileged_vm/run.py --memory-mb 4096`), nothing else
touched: no push, no Pi, no remote host. The scripts under test are the real `scripts/cutover/{preflight,activate,return}.sh`
and `limeosctl migrate {scan,import-users,propose-ceiling}`; until now they had only run against stubs. Source: local main
`fc59411` plus the `return.sh` fix `e1b243e`; packages `0.4.17+cutover.1` built inside the guest from that bundle (the build
step is not in this packet). Guest script: `tests/privileged_vm/p06_cutover_guest.py`. Final run: [result.json](raw/result.json)
(16 cases, all pass), raw script transcripts in [raw/logs](raw/logs).

## What the guest imitates

- Frozen app: `pi-health.service` (python HTTP on `0.0.0.0:8002` as `holly` from `/home/holly/pi-health`, with
  `config/backup_config.json`, `media_paths.json`, `storage_plugins/`), `pihealth-helper.service` (root, `PartOf=pi-health`),
  `limeos-metrics-collector.timer` (every 20 s, writes a SQLite file) and `limeos-package-reconcile.timer`.
- `/etc/limeos/credentials.env` (0640 root:holly) with `PIHEALTH_USERS=holly:<Werkzeug scrypt>,bob:<Werkzeug PBKDF2>` (the real
  hashes in `tests/fixtures/werkzeug-hashes.json`), `agent-policy.json`, policy backups, and `/var/lib/limeos` JSON and four
  SQLite files like wybie's.
- Bystanders that must never change: `limeops-actuatord` (ordered after the helper), `limeops-worker`, `limeos-agent`, `flask-shutdown`.
- Real Docker Engine (Debian `docker.io` 20.10.24, stock **socket-activated** listener, peer PID 1), `holly` in the `docker` group,
  seven containers created by `holly`: compose project `media` (jellyfin on 8096 via docker-proxy, radarr, sonarr, transmission
  running; airsonic-advanced stopped) plus standalone `watchtower` and `vpn`. `/opt/stacks/media` owned by `holly`.
- Caddy 2.6.2 from Debian, installed, inactive and disabled; its Caddyfile is only `import /etc/caddy/conf.d/*.caddy`. Public site
  `https://wybie.lan:8002` with Caddy's internal CA (the guest verifies the certificate against that CA).

## Cases

| # | Case | Result | Observed |
| --- | --- | --- | --- |
| 0 | Fixture | pass | frozen app serves 8002 as holly, units enabled and active, 6 running + 1 stopped container, caddy inactive |
| 1 | Package install from the signed repo | pass | frozen app, containers, bystanders and every frozen file hash unchanged |
| 2 | `migrate scan` | pass | exit 0, names every expected source (credentials.env imported; SQLite, JSON settings, integrations, storage_plugins deferred; the rest unsupported), no secret hash or password in the report, originals unchanged |
| 3 | `import-users` | pass | imported 2; scrypt user logs in through the real API on 8003 and upgrades to Argon2id; the PBKDF2 user stays legacy until its own login; wrong password 401; re-run `imported: 0 ... nothing changed, skipped existing: 2` and no hash changed; a file with a bad second entry imported nothing (the valid first entry too) |
| 4 | `propose-ceiling` + operator enable | pass | six containers and `managed_stacks=[media]`, every `allow_*` false, `check-ceiling` accepts; installed as proposed, a restart plan is refused (400); after the scripted enable step (health, container read/logs, restart/start/stop, stack) it is accepted |
| 5 | Preflight refusals | pass | stopped managed container and Jellyfin `NowPlayingItem` each refuse (exit 1), `activate.sh` rejects the refused snapshot; nothing changed |
| 6 | Cycle 1: preflight, `activate --dry-run`, `activate` | pass | dry-run changes nothing; real activate 4.6 s: frozen units stopped+disabled, Rust units and caddy active+enabled, 8002 is Caddy, 8003 is the core, plain HTTP on 8002 no longer answered; container IDs, StartedAt and PIDs identical; bystanders (PID, InvocationID, enter timestamp) identical; frozen-file hashes identical; frozen SQLite quiescent for 25 s |
| 7 | HTTPS through Caddy | pass | PBKDF2 user logs in over HTTPS and upgrades; Argon2id login; wrong password 401; cookie is Secure; nosniff and CSP headers; overview lists all 7 containers with status; jellyfin logs served; unmanaged (stopped) container logs 403; host/disk/partition resources present. Stack status is `unavailable` here (see Findings 3) |
| 8 | `return.sh` after the clean activation | pass | exit 0 "return verified"; frozen units enabled+active, caddy stopped+disabled and site removed, Rust units stopped+disabled, 8002 plain HTTP as holly; containers, bystanders and hashes identical to the snapshot; second run also exit 0 |
| 9 | Cycle 2: preflight + activate again | pass | re-activation with the package installed and users imported |
| 10 | Approved container restart (HTTPS plan, approve, queue) | pass | radarr restarted in place (same ID, new StartedAt); Engine events only for radarr; others unchanged |
| 11 | Whole-stack restart `media` | pass | after `systemctl start limeos-storage-reader` (Findings 3): job succeeded; restart events for exactly the four running members; stopped member and standalone containers untouched |
| 12 | `return.sh` after those restarts | pass | exits non-zero naming "container StartedAt differs" (the restarts are detected) but still restores the frozen app, and stops the hand-started stack reader |
| 13 | Failed activation: proxy upstream wrong, health check fails | pass | `FAIL: health check failed: core health via the proxy`, automatic `return.sh` "return verified"; frozen app serves 8002 as holly, Caddy site gone, caddy and Rust units stopped, containers/bystanders/hashes identical; `return.sh` run again exit 0 |
| 14 | Failed activation: wrong `--expected-version` after the package step | pass | same rollback and verification |
| 15 | Refusals | pass | no approval flag, wrong host (origin mismatch), stale snapshot all refused before any change |

## Bugs found and fixed (commit `e1b243e`, regression tests in `tests/cutover`)

1. **`return.sh` failed after every clean activation and every rollback** with
   `live state file changed after the writers stopped: /var/lib/limeos/metrics/metrics.sqlite`. It restored the frozen units
   first and hashed the state afterwards; the restored timer fires at once and rewrites its SQLite file, which the post-stop
   baseline then reports as a late writer. This would likely also have hit the Pi (metrics timer, SQLite files) and made the
   automatic rollback report "ROLLBACK INCOMPLETE". Fix: hash the state before the frozen writers restart; if a frozen writer is
   already running (second run of `return.sh`), live-database differences only warn, strict files still fail. Tests:
   `test_hashes_are_verified_before_the_frozen_writers_restart`, `test_second_return_tolerates_live_databases_but_not_strict_files`
   (both fail on the old script).
2. `return.sh` left `limeos-storage-reader` running if an operator had started it for stack status (Findings 3). It now stops it
   when active (never enables it). Test: `test_stops_a_hand_started_stack_reader_but_not_otherwise`.

Documentation corrected: `propose-ceiling` needs `docker ps --format '{{json .}}'` on Docker older than 23 and its output is
inert including `allow_health`/`allow_container_read`; runbook notes the reader and catalog gaps.

First run (before fixes) had 9 failing cases, all consequences of the two script bugs above plus my own guest mistakes (feeding
the deny-by-default proposal to the executor health probe, a pre-activation unit assertion); the second run passed all 16 with
fix 1; the final run passed with both fixes.

## Findings that need an operator or design decision (not changed)

3. **Stack status and whole-stack actions are unavailable after `activate.sh`.** `/api/v1/deployment/stacks` reports catalog and
   Engine `unavailable` until the dormant `limeos-storage-reader.service` (root, read-only, deliberately no `Install` section)
   is started by hand, and the reader also needs a root-owned `/etc/limeos/management-catalog.json` (1 to 8 stacks, each with
   the Compose file's SHA-256). No tool or script writes the catalog or starts the reader; the guest writes the catalog as an
   operator step. Whether `activate.sh` should start the reader is a design decision, so it was not changed.
4. `propose-ceiling` writes `allow_health=false`; installed verbatim, the executor refuses its health probe. The operator step must
   enable at least `allow_health` and `allow_container_read` (documented now).
5. The stock socket-activated Docker listener (peer PID 1) works for container reads, logs, approved restart and stack restart.

## Stubbed or not representative of the Pi

- x86_64, 4 KiB pages (the preflight records the page size but cannot exercise the 16 KiB case), Debian 12 kernel 6.1 cloud
  kernel, Docker 20.10.24 from Debian (the Pi's Docker is likely newer; API versions untested), Caddy 2.6.2 from Debian.
- No swap in the guest, so the swap-pressure refusal is not exercised (the stub tests cover it); MemAvailable 4 GiB total
  passes the 1.5 GiB gate.
- Frozen app, helper, collector and bystanders are tiny imitations; Jellyfin is a busybox shell loop, so playback is
  unobservable (the script warns, as on wybie) and the refusal case uses a local fake `/Sessions`. Seven containers instead of
  eighteen; no real Compose, media paths or watchtower behaviour.
- The management catalog and the ceiling enable step are scripted by the guest (no product tool exists for the catalog).
- Browser use (trusting the Caddy CA, UI) is not covered; HTTPS was exercised by a Python TLS client against Caddy's internal CA.
- `activate.sh` re-installs the already installed `.deb` (postinst restarts the Rust services); on the Pi the package may be new.

## Reproduce

```
python3 tests/privileged_vm/run.py --image .../debian-12-genericcloud-amd64.qcow2 --checksums .../SHA512SUMS \
  --repository BUILD/limeos-repo --memory-mb 4096 \
  --guest-script tests/privileged_vm/p06_cutover_guest.py --output result.json
```

`run.py` now also copies `scripts/cutover` and `packaging/Caddyfile.cutover.example` to `/root/limeos-src` in the guest, accepts
`--memory-mb`, and keeps a failed guest's `result.json` as `*-failed.json`.

Cleanup: after the final run no `qemu-system` process exists and no `/tmp/limeos-p01-vm-*` directory remains.
