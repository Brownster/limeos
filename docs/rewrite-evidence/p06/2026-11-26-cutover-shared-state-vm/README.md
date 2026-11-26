# Cutover rehearsal with wybie's shared state directory (VM, 26 Nov 2026)

`tests/privileged_vm/p06_cutover_guest.py` in one disposable Debian 12 amd64 KVM guest (systemd 252,
bash 5.2, 2816 MiB, `tests/privileged_vm/run.py`, under the machine's heavy-job lock), after the first wybie
attempt ([evidence](../2026-11-26-wybie-first-attempt/README.md)). The fixture now has wybie's units:
pi-health and its metrics collector own `/var/lib/limeos` through `StateDirectory=limeos` as
`holly:pihealth 0750`; a limeops writer keeps writing `supervision.sqlite3`; the agent supervisor's
virtualenv is deeper than the scan's bound; `docker-compose-start.service` has been failed since boot;
the Compose projects include one outside `/opt/stacks`. The scripts run from the directory `bundle.sh`
builds, copied as on the host.

Packages `0.4.17+cutover.1` and `.2` were built locally with `packaging/build.py` from the branch's
packaging and the binaries of a CI build, then put in a test repository signed with a disposable key.

## Runs

| Run | Binaries | Scripts, postinst | Result |
| --- | --- | --- | --- |
| [branch](EVIDENCE-BUNDLE.json "bundled: raw/branch-binaries/result.json") | CI run 45745042420 (`pr-cutover-shared-state` at `d4d108a1`, artifact `candidate-debian-ubuntu-24.04`; [hashes](branch-binaries.sha256)) | guest `3a57044b`, scripts and postinst `d4d108a1` | 26 of 27 cases pass; case 24 fails as expected |
| [old binaries](EVIDENCE-BUNDLE.json "bundled: raw/old-binaries/result.json") | CI run 45513269776 (0.4.4) | guest and scripts `3a57044b`, postinst `b2f8dad3` (owner compared with 0, not EUID: the same under dpkg) | 25 of 27 pass; case 2 (scan without the virtualenv change) and case 24 fail as expected |

## What the runs show on systemd 252

- **Configure on a real dpkg** leaves `/var/lib/limeos` `holly:pihealth 0750`, creates LimeOS's trees
  inside it with LimeOS's owners, neither enables nor starts the services, and says so (case 1).
- **No re-chown when the owner already matches.** A restart of pi-health with LimeOS installed leaves
  LimeOS's trees with their accounts (case 1). The same holds after every return, which gives the
  directory back as `holly:pihealth 750` before the frozen app starts (cases 8, 12-14, 17-23).
- **The guard drop-in refuses a start.** After activation, `systemctl start pi-health.service` and
  `systemctl start limeos-metrics-collector.service` fail on the assertion; `/var/lib/limeos` stays
  `root:root 0755` and core keeps answering (case 6).
- **Returns verify despite the limeops writer.** Every return reports `supervision.sqlite3` as live
  state that changed and ends `return verified`, leaving LimeOS stopped and disabled with the reason;
  core's database and its users survive (cases 8, 13 and the forced returns).
- **The scan** on the branch binaries lists the virtualenv as one entry and is complete (case 2); the
  runbook's `state_lists.py` builds a list with no virtualenv file and no live path (case 4a).
- **Case 24, wybie's container mix** (18 containers, 14 running, 3 with passing health checks, 2 stopped
  after a failed check, 51 mounts), reproduces wybie's 26 Nov failure on both builds: core's
  container-health read through the reader fails with `the read failed: Unavailable`, activation rolls
  back, and the rollback's return verifies. The fix is on branch `pr-health-cohort`; this case needs a
  build with it merged.

## Files

`branch-binaries.sha256` (tracked). Bundled: each run's `result.json` (cases, facts, script
transcripts) and `run-output.log.gz`.
