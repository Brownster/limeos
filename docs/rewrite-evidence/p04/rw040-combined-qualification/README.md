# RW-040 combined disposable-guest qualification: handoff

| Item | Value |
|---|---|
| Branch | `engineer/rw040-confinement-qualification` |
| Worktree | `/home/marc/Documents/github/lime-os-rw040-confinement` |
| Base | `2fd74209e1238c1374837ab431ef670020726dc2`; its full CI, [run 37746766303](https://github.com/Brownster/limeos/actions/runs/37746766303), succeeded on that exact SHA before work began |
| Guest qualification run | `20261008T165447Z-7f86e022` at harness commit `0358017`, clean tree |
| Recorded checks | [logs/checks.json](logs/checks.json) at `7ef5894` |
| Brief | `docs/plans/2026-10-08-engineer-rw040-confinement-qualification.md`, on main after this base |

**No P04 gate closes, and no RW-040 library result is claimed.** The base has no
`bins/executor/tests/combined_read_probe.rs`, and the integrator owns it. Every
case that needs the probe or the later worker is reported as `blocked` (17 of
17). This branch delivers the harness, fixtures, independent ground truth,
confinement reproduction, environment baselines and a draft probe contract. A
probe-run verdict remains future work.

## What ran

One owned Debian 12 AMD64 KVM guest ran on this workstation: 2 vCPUs, 1,536 MiB,
4 KiB pages, guest kernel 6.1.0-53-cloud-amd64 and systemd 252.39. It was booted
by QEMU 10.1.5 on Fedora 43 (host kernel 7.0.14).
- **Transport:** console capture plus QMP. There was no SSH, host forward, host
  share or workstation Docker use.
- **Duration and teardown:** 174 seconds. The guest powered itself off with QEMU
  exit 0; the deadline wasn't reached and no QMP quit or kill was needed. The run
  directory was removed and no marker process survived (`guest-run/run.json`).
- **Inputs, verified before boot and again in the guest:**
  - Debian image SHA-512 `a09170d1…5f0882c`, checked against Debian's current
    `SHA512SUMS`
  - the exact-source CI artifact `debian-ubuntu-24.04`, zip `1e90f746…`, whose
    three packages match the integrator's recorded hashes
    ([record](../../../../tests/fixtures/rw040-combined/packages-2fd7420.json))
  - the inputs disk, SHA-256 `bce820b1…`
- **Installed package:** `limeos_0.4.4_amd64.deb`. Its four binaries equal the
  recorded hashes, and its storage-reader unit is byte-identical to
  `packaging/systemd/limeos-storage-reader.service` at the base (`b5feec64…`).
- **Engine:** Debian `docker.io` 20.10.24, with containerd 1.6.20 and runc 1.1.5.
  The container image is a local import of `busybox-static`; nothing was pulled.

All 14 guest stages passed. Their durations:

| Stage | Seconds |
|---|---|
| install | 77.9 |
| budget | 50.4 |
| containers | 9.9 |
| transitions | 4.8 |
| layout | 3.4 |
| selfcheck | 3.2 |
| everything else | under 1 each |

[summarize.py](summarize.py) derives [summary.json](summary.json) mechanically from
the raw output in [guest-run/](guest-run/), and the summary reproduces exactly.

## Results that are qualified here

| Evidence | Result |
|---|---|
| **Confinement reproduction** | A transient unit carrying the installed unit's 21 non-lifecycle `[Service]` settings equals the installed `limeos-storage-reader.service` on all 62 compared `systemctl show` properties: 23 derived from the unit's settings and 40 sandbox defaults, with `CapabilityBoundingSet` in both ([equivalence](guest-run/guest/confinement-equivalence.json)). Nothing is granted or raised. |
| **Ceilings enforced** (harness self-check, not a probe) | Descriptors: 253 opened beside 3 inherited, stopping at 256 with EMFILE; the sampler peak was 256, against 303 unrestricted. Tasks: 15 threads plus the main thread (`TasksMax=16`), against 24. Memory: OOM-killed with the cgroup at exactly 67,108,864 bytes after touching 56 MiB, against 96 MiB unrestricted. Each transient unit was inactive afterwards ([self-check](guest-run/guest/selfcheck/ceilings.json)). |
| **Engine endpoint facts** | `/var/run` is a symlink to `/run`. The canonical `/run/docker.sock` is `root:docker 0660`, socket-activated by `docker.socket`. **`SO_PEERCRED` reports PID 1 (systemd)**, not `dockerd` (PID 964), so peer credentials identify the activating listener, not the daemon. Without symlink resolution, `openat2` of `/var/run` gives ELOOP. |
| **Membership ground truth** | Empty membership, then 14 consumers with 12 running: bind, alias, symlink, file, named-volume, tmpfs, swappable-device, mountless, cross-UID, nondumpable and transition consumers, plus stopped bind and mountless consumers. Captured from the Engine API, `/proc`, `findmnt`, `lsblk` and `blkid` ([ground truth](guest-run/guest/ground-truth/)). |
| **Nondumpable fixture** | Verified for the UID 1990 container running a setuid-root busybox copy: its `/proc` entries are owned by root. Other consumers are dumpable. |
| **Transitions** | Each verified with independent facts ([transitions](guest-run/guest/transitions/verification.json)): same-PID root switch, same-PID mount-namespace switch, restart with a new PID, exit to PID 0, a device swapped under a fixed host path (UUID changed while the container's table stayed unchanged), and a Btrfs consumer, created and removed. |
| **Membership scale** | 65 consumers (62 running) were created and removed. Guest `MemAvailable` fell from 1,018 MiB to 635 MiB ([feasibility](guest-run/guest/budget/feasibility.json)). Probe headroom at that scale is still unmeasured. |

## Findings the integrator needs before the probe

These come from `environment/baseline.json`. They are kernel-interface outcomes
from standard tools run as unrestricted root and under the reproduced
confinement. They are **not** library results, but they predict fail-closed
library outcomes.

| Interface | Unrestricted root | Unchanged reader confinement (UID 0, GID 114 `limeos-host-access`, no capabilities, `NoNewPrivs`, seccomp) |
|---|---|---|
| `openat2` with `RESOLVE_NO_SYMLINKS` | Succeeds | **ENOSYS** |
| `statx` with mount ID | Succeeds | Succeeds |
| `pidfd_open`; reading `/proc/PID/status` and `mountinfo` | Succeed for every target | Succeed for every target |
| `/proc/PID/ns/mnt`, `/proc/PID/root` (open, readlink, stat, statx) | Succeed for all 16 targets: PID 1, 12 containers, 3 host tasks | **EACCES for all 16 targets** |
| Canonical Docker socket `/_ping` | 200, peer PID 1 | 200, peer PID 1 |
| `AF_INET` socket | Created | EAFNOSUPPORT |

Two single-setting diagnostic variants attribute these refusals. They remove one
existing setting each, grant nothing, and are never qualification modes.

**`openat2` → ENOSYS is caused by `RestrictSUIDSGID=yes`.**
- Removing only that setting makes `openat2` succeed.
- Removing only `Group=` doesn't.
- `openat2` appears in the expanded `@system-service` allowlist, so the filter's
  allowlist isn't the cause.

The Engine and process libraries require `openat2` and refuse without it, so
every confined combined request should refuse until the integrator decides this
policy.

**`/proc` namespace and root access has two independent blockers.**
- *GID mismatch:* with `Group=` removed (GID 0), only the dedicated UID-0 task
  that holds no capabilities becomes inspectable.
- *Empty capability set:* every Docker container (they hold capabilities), PID 1,
  the nondumpable task and the other-UID task stay EACCES even at GID 0. This
  matches the kernel's ptrace access rules (no `CAP_SYS_PTRACE`, capability
  subset, dumpability and UID checks).

Even without `Group=`, running-container process evidence is unavailable without
a capability decision. These policy choices belong to the integrator; this
branch changes no unit, capability or ceiling.

## Acceptance matrix

Every case in [cases.json](../../../../tests/fixtures/rw040-combined/cases.json)
is **blocked**. The guest reports which stages prepared each one
([cases](guest-run/guest/cases.json)).

| Case | Missing prerequisite | Prepared by |
|---|---|---|
| engine-empty, engine-complete, engine-endpoint-facts | integrator probe | empty, containers, ground-truth |
| process-same-uid, process-cross-uid, process-nondumpable, kernel-support-missing | integrator probe | containers, environment |
| source-aliases-nested-volumes, source-swapped-device, source-unsupported-backing | integrator probe | layout, containers, ground-truth, transitions |
| transition-restart-exit, transition-root-namespace, transition-expiry | integrator probe | transitions, containers |
| budget-empty-near-over | integrator probe | budget |
| budget-concurrent | integrator probe and worker | selfcheck |
| worker-cancel-death-deadline | integrator worker | selfcheck |
| worker-blocked-synchronous | integrator worker and a blocking fixture (not built) | none |

[probe-contract.md](../../../../tests/fixtures/rw040-combined/probe-contract.md)
proposes the interface: hash-bound delivery, modes, environment, a transition
handshake and output. The runner's `--probe` option refuses until both sides
agree.

## Failed attempts, preserved

| Attempt | Source | Result |
|---|---|---|
| [1](attempts/attempt-1/) | Uncommitted harness; hashes in `run.json` | `mkfs.xfs` refused the 128 MiB disk, because xfsprogs 6.1 needs at least 300 MB. Layout and dependent stages were skipped. The descriptor self-check also crashed when it ran out of descriptors to list `/proc/self/fd`, a harness bug. Fixed with a 320 MiB disk and baseline counting. |
| [2](attempts/attempt-2/) | Uncommitted | The namespace switch failed. cloud-init rejected `ssh_genkeytypes: []`. Diagnosis added `docker logs` capture. |
| [3](attempts/attempt-3/) | Uncommitted (`source_dirty: true`) | busybox `unshare -m` was denied a mount by Docker's default AppArmor profile. Fixed by running only that observed fixture container without AppArmor. Attribution diagnostics were added in this attempt. |
| [checks 1](checks-attempt-1/) | `c82a0da` | `git diff --check` found trailing whitespace in `probe-contract.md`, and Ruff found an unused `noqa` |
| [checks 2](checks-attempt-2/) | `9fb944e` | `git diff --check` re-flagged the whitespace quoted inside the preserved failed log. Fixed by a scoped `.gitattributes`; the raw log is unchanged. |

Attempts 1–3 ran the base's exact packages and image; only harness and fixture
files differed. Their console logs are gzipped.

## Checks

At `7ef5894` ([logs](logs/), [checks.json](logs/checks.json)), all passed:
- 17 harness tests, with `ResourceWarning` treated as an error
- Ruff lint and format on the harness and evidence scripts
- `scripts/check_repository.py`
- `scripts/check_contracts.py`
- `git diff --check` from the base `2fd7420`
- an ownership review: 113 changed paths, none outside the owned directories

No Rust, lockfile, workflow, packaging or production source changed. The fmt,
Clippy, workspace-test, deny and audit gates therefore have no supplied seam to
qualify, and they weren't rerun here; the base's CI covers them. The base's 15
reference and 33 ARM harness tests weren't rerun locally.

## Not tested or unresolved

- **Probe results:** every RW-040 library result, unrestricted or confined,
  awaits the integrator's probe.
- **Worker:** single-flight admission, concurrent requests, cancellation, parent
  death, independent deadlines and blocked synchronous work await the worker.
  No blocking-filesystem fixture exists.
- **Headroom:** real descriptor, RSS, PSS, read-traffic and latency headroom for
  combined collection is unmeasured. The 261/265-descriptor and 65.78 MiB
  fixture figures remain the libraries' own records.
- **Policy decisions:** the `openat2` and ptrace findings need integrator policy
  decisions. Stricter variants (hidepid, statx denial) are described in
  `cases.json` but weren't run.
- **Platforms:** ARM64, the Pi, bare metal, installed combined service and CI
  execution of this harness are untested. The [proposed CI step](../../../../tests/qualification/rw040-combined/README.md#proposed-ci-invocation-for-integration)
  is for the integrator to add.
- **Historical row:** the 61.401 ms / 20 ms storage-inventory row remains failed.

## Effort

| Item | Value |
|---|---|
| Estimate | 16 human engineering hours; review at 24 |
| Agent time | One session on 2026-10-08, from 15:28Z to about 17:05Z. That's elapsed agent time, not human hours. |
| Infrastructure waits | Four guest runs of 1.5–3 minutes each; image (350 MB) and artifact downloads; three check runs of about 30 seconds each. No approval waits. |
| Human time | None measured |

[SHA256SUMS](SHA256SUMS) covers this directory's files and excludes itself.
