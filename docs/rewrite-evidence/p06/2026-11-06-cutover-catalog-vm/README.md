# Cutover rehearsal with `propose-catalog` and the activate-started stack reader

One disposable Debian 12 AMD64 KVM guest (`tests/privileged_vm/run.py --memory-mb 4096`), local only: no push, no Pi, no
remote host. Same fixture and 16 cases as the [first rehearsal](../2026-11-06-cutover-rehearsal-vm/README.md) plus case 4b.
Source: local main `6c5789e` plus commits `ce61b75` (propose-catalog), `46b582b` (scripts and stub tests), `8beb731` (docs),
`0848039` and `09bfffb` (guest). Packages `0.4.17+cutover.1` built inside the guest from a bundle of that tree.
Result: [raw/result.json](raw/result.json), 17 cases, all pass; transcripts in [raw/logs](raw/logs), runner output in
[raw/run-output.log.gz](raw/run-output.log.gz).

## What changed in the guest

- The hand-written `/etc/limeos/management-catalog.json` is gone. **Case 4b** (new): preflight without a catalog refuses and prints the
  exact `limeosctl migrate propose-catalog ... --install` command and the `install` alternative; `propose-catalog --out` lists
  `stack media (project media) ... services=airsonic-advanced,jellyfin,radarr,sonarr,transmission`, ignores the 2 standalone
  containers, writes exactly the expected catalog (engine ID from `docker info`, Compose file SHA-256, `managed` absent), does
  not install it, refuses to overwrite its output; `--install` installs it root:root 0644 and a second `--install` refuses to
  replace it. The reader is not running before the cutover.
- **Case 6**: `activate.sh` (6.7 s) prints `ok: stack reader ready` and `ok: stack reader still ready after settling`, and leaves
  `limeos-storage-reader` active and not enabled. **Case 7**: right after activate and with no manual step, the first poll of
  `/api/v1/deployment/stacks` (0.0 s wait) shows catalog and Engine `fresh`, `engine_matches_catalog` true, `media` with 5 ordinary
  members, 4 running, no issues.
- **Case 11**: the whole-stack restart of `media` runs without the old manual `systemctl start`; the guest asserts the reader was
  already active. Restart events are exactly the four running members.
- Frozen-state checks in cases 5, 8, 12, 13, 14, 15: reader stopped and not enabled after refusals, returns and both automatic
  rollbacks (health check failure, wrong version).

First run failed only case 5 (a guest helper expected the reader state to follow `rust_running`; it is never started before
activate). Fixed in `09bfffb`; this packet is the second, fully passing run.

## Not covered

Same limits as the first rehearsal (x86_64, Docker 20.10.24, Caddy 2.6.2, 7 containers). Not exercised in the VM, only by unit and
stub tests: wybie's `docker` project outside the root, one-off containers, more than 8 stacks or 16 services, a stack
directory with no containers, and a reader that never listens or crash-loops (the stub tests inject those).

Cleanup: after the run no `qemu-system` process and no `/tmp/limeos-p01-vm-*` directory remain.
