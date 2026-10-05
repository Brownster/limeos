# Native ARM64 qualification report

Prepared: 2026-10-06, overnight on the reference Pi 5 (wybie), with the operator's permission to use it as a hypervisor while idle. Brief: [Engineer 1: native ARM64 qualification](../../plans/2026-10-05-engineer-arm64-qualification.md). Scripts and reproduction steps: [`tests/qualification/arm64/`](../../../tests/qualification/arm64/README.md).

## Summary

Two payloads were qualified separately, on native ARM64 hardware:

| Payload | Source | Result |
|---|---|---|
| Frozen 0.4.2 | `589cf9a` | Builds, packages, installs and meets every measured budget. **Fails one native test:** configuration reads follow symlinks on ARM64. Two more opens have the same defect but no test. |
| Corrected 0.4.2 | `5848bce` = `589cf9a` + cherry-picked `1087f24` | Every native gate passes (132 of 132 tests). Also passes: the P04 storage-target suite on guarded virtual disks (35 of 35), package identity, real upgrades from the genuine 0.1.1 and 0.3.2 arm64 packages, and the command-time and storage-reader measurements. |

The defect was hard-coded x86-64 open flags. `0x20000` is `O_NOFOLLOW` on x86-64, but `O_LARGEFILE` on arm64, where `O_NOFOLLOW` is `0x8000`. On a Pi, LimeOS therefore followed symlinks when reading configuration, opening `core.lock` and opening the container executor's `receipts.lock`. The independent correction `1087f24` takes both flags from the already-pinned `rustix`.

## Hardware and method

| Item | Value |
|---|---|
| Hypervisor | Raspberry Pi 5 Model B Rev 1.0: 4 × Cortex-A76 (CPU part `0xd0b`), 8 GB RAM, KIOXIA 256 GB NVMe. Host kernel 6.12.25+rpt-rpi-2712 with 16 KiB pages. |
| Virtualisation | QEMU 7.2.22 with KVM (`-accel kvm -cpu host`, `virt` machine, AAVMF 2022.11), at `nice 19` and idle I/O priority. QEMU opened `/dev/kvm` as root, then dropped to the SSH user with `-runas`. |
| Guest | Debian 12 genericcloud arm64 (published 2026-09-23, SHA512 checked), kernel 6.1.0-53-cloud-arm64 with 4 KiB pages. `systemd-detect-virt` reports `kvm`. |
| Guests used | Build: 3 vCPU, 2.5 GiB (frozen) or 2 GiB (corrected). Install, storage and upgrade: 2 vCPU, 1.5 GiB. Each guest's system disk was a fresh virtio-blk qcow2 overlay (24 GiB for builds, 16 GiB otherwise) on the Pi's NVMe, destroyed afterwards. The storage guest also had four empty 128 MiB qcow2 disks. |
| Host activity | The Pi's own 18 containers, including Jellyfin and the Python LimeOS, kept running. The operator reported no overnight use. Sampled host load averages ranged from 0.05 to 1.41. |

Native architecture is proved independently three ways: `uname -m` is `aarch64` in the guest and on the host; the guest CPU reports implementer `0x41` and part `0xd0b` (Cortex-A76) under KVM with `-cpu host`, which cannot run under emulation; and every installed binary's ELF header is `AArch64`. Neither cross-compilation nor emulation produced any of the results here.

The frontend is plain JavaScript, so it was built on the workstation from the same commit and its file hashes recorded. All Rust was compiled natively in the guest.

## Frozen 0.4.2 (`589cf9a`)

Evidence: [`2026-10-06-frozen-589cf9a/`](2026-10-06-frozen-589cf9a/).

### Build and gates

| Gate | Result |
|---|---|
| Toolchain | rustc 1.88.0 (6b00bc388 2025-06-23) via rustup; native linker `/usr/bin/aarch64-linux-gnu-gcc` (Debian gcc 12), as named in `.cargo/config.toml` |
| `cargo fetch --locked`, `cargo fmt --check` | Pass |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | Pass (72 s) |
| `cargo test --workspace --locked --no-fail-fast` | **Fail: 130 passed, 1 failed** across 27 targets. `config::tests::public_cleartext_and_symlink_config_are_rejected` fails at `crates/persistence/src/config.rs:244`, because reading a symlink to `/etc/passwd` succeeds. |
| `scripts/check_contracts.py` | Pass: generated contracts match on ARM64 |
| Release build of the four binaries | Pass (247 s, 2 jobs) |
| Standard and shadow `arm64` packages, signed test repository | Pass. `limeos_0.4.2_arm64.deb` `08b06164…69c06`; `limeos-shadow_0.4.2_arm64.deb` `da5e3c23…84df7` |

Two regression tests reproduced the defect in the other opens on the same native build: a symlinked `core.lock` and a symlinked `receipts.lock` were both followed ([log](2026-10-06-frozen-589cf9a/fix-verification/repro-frozen-with-new-tests.txt)).

### Installed behaviour

A clean guest installed `limeos 0.4.2 arm64` from the signed repository. 20 checks ran ([result](2026-10-06-frozen-589cf9a/install/install-result.json)); all passed after one harness rerun:

- **Package identity:** `dpkg --verify` is clean; the installed binaries match the native build's hashes, and the repository packages match the build. The first attempt failed only because the harness called `readelf`, which a clean guest lacks. The fixed harness reads the ELF header directly and [passed in the same installed guest](2026-10-06-frozen-589cf9a/install/identity-rerun.json).
- **Units:** `limeos-core`, `limeos-containerd` and `limeos-storaged` are enabled and running. `limeos-storage-reader` and `limeos-storage-ready` are static and start on demand.
- **Accounts and capabilities:**

  | Service | Runs as | Notes |
  |---|---|---|
  | Core | `limeos-core` (UID 104) | |
  | Container executor | `limeos-containerd` (UID 105) | |
  | Host executor | root | empty capability bounding set |

  For all three, `CapEff`, `CapBnd` and `CapAmb` are zero and `NoNewPrivs` is 1.
- **Ownership:** all 13 root-executed paths (binaries, linked libraries and their parent directories) are root-owned and not group- or world-writable.
- **Docker:** the core account gets `EACCES` (errno 13) on `/run/docker.sock`. Only the container executor's process carries the `docker` group.
- **Enrollment:** there's no default login. Local enrollment works once; a second bootstrap is refused.
- **Imported passwords:** a Werkzeug scrypt hash (first login 166 ms) and a PBKDF2-SHA256 hash with 1,000,000 iterations (first login 316 ms) both verify. Wrong passwords are refused, and both hashes upgrade to Argon2id, after which logins take 65 ms.
- **Password worker:**
  - At most 2 workers ran at once (core's permit limit). Of 10 logins, including a burst of 6 concurrent, 6 succeeded and 4 got 429 because both permits were busy; they were refused, not queued.
  - Each worker peaked at 20.7 MiB and exited, and core returned to 5.1 MiB PSS.
  - Login latency was p50 71 ms, maximum 153 ms.
- **Sessions:** a session survived restarting core, the container executor and the host executor.
- **Shadow:** it installs beside the standard package, listens on port 8004, and its executor carries the `read-only` argument. It refused restart-, start- and stop-enabled ceilings, each ending in `Result=exit-code`. Removing it left the standard core's PID and session unchanged.

### Measurements against the architecture budgets

The workload was 18 running containers (a static busybox fixture on Debian `docker.io` 20.10.24), one authenticated user, and either no subscribers or 3 open dashboard streams. Raw values: [`install-result.json`](2026-10-06-frozen-589cf9a/install/install-result.json); computed table: [`budgets.json`](2026-10-06-frozen-589cf9a/budgets.json).

| Measurement | Measured | Budget | Within |
|---|---|---|---|
| Core and base executors, idle PSS | 9.68 MiB (core 6.99, container executor 1.48, host executor 1.21) | 30 MiB | yes |
| The same, after the latency workload and shadow removal | 10.10 MiB | 30 MiB | yes |
| All resident LimeOS processes (no assistant exists in 0.4.2) | 9.68 MiB | 60 MiB | yes |
| Swap used by LimeOS services | 0 | none in steady state | yes |
| Idle CPU, 600 s, no subscribers | 0.045% of one core | under 0.1% | yes |
| CPU with 3 dashboard streams, 300 s | 0.082% of one core | none set | — |
| Bytes written by resident LimeOS processes | 32 KiB in 600 s idle and 16 KiB in 300 s with dashboards, both about 4.7 MB/day extrapolated; executors 0 | under 20 MB/day | yes |
| Core restart to ready (20 restarts) | p50 237 ms, p95 267 ms, maximum 277 ms | under 1 s | yes |
| Full stack cold start to ready (5 starts) | p50 450 ms, maximum 463 ms | under 1 s | yes |
| Cached overview, sequential (20 requests) | p50 0.91 ms, p95 1.00 ms | under 20 ms | yes |
| Authenticated reads, 3 concurrent clients × 200 requests | p95 3.9 ms, p99 up to 8.6 ms (history), 1,182 requests/s | under 20 ms | yes |

RSS for the same idle point: core 8.36 MiB, container executor 3.57 MiB, host executor 3.30 MiB.

Measured separately, outside the LimeOS figures:

| Component | PSS |
|---|---|
| Password worker, during login only | up to 20.7 MiB per worker, at most 2 at once |
| Root storage reader (on demand) | measured on the corrected payload, below |
| Docker engine | `dockerd` 79.8 MiB, `containerd` 49.2 MiB, 18 `containerd-shim` processes 142.6 MiB |
| Media workload | 18 busybox processes, 2.1 MiB in total |
| Assistant runtimes | not present in 0.4.2 (RW-040 is in progress), so not measured |

Docker's own idle CPU was 0.28% of one core (`containerd`) and 0.07% (`dockerd`).

How writes were counted: the cgroup `io.stat` files exist in the guest but reported no write bytes for any LimeOS unit, so they weren't used. The figures above come from each service's `/proc/<pid>/io` `write_bytes`. Whole-guest disk writes, including Docker and journald, were 8.6 MB in the 600-second idle window. Ten-minute windows extrapolated to a day are indicative only; the architecture's 24-hour soak remains open.

## Corrected 0.4.2 (`589cf9a` + `1087f24`)

Evidence: [`2026-10-06-corrected-5848bce/`](2026-10-06-corrected-5848bce/). Source `5848bce` is `589cf9a` plus `git cherry-pick -x 1087f24`, which applied without conflicts. The runtime change is three open-flag expressions in `limeos-persistence` and `limeos-executor-container`, plus a regression test for the database lock. Nothing in the integrating branch's schema-7 lock work is included.

### Build and gates

| Gate | Result |
|---|---|
| `cargo fetch --locked`, `cargo fmt --check`, strict Clippy, `scripts/check_contracts.py` | Pass |
| `cargo test --workspace --locked --no-fail-fast` | **Pass: 132 of 132** across 27 targets: the 131 frozen tests plus `symlinked_core_lock_is_refused_without_creating_authority_state`. The config symlink test now passes. |
| Release build, standard and shadow packages, signed repository | Pass. `limeos_0.4.2_arm64.deb` `0a8651aa…6e51`; `limeos-shadow_0.4.2_arm64.deb` `31ba9929…7b5d`. Binary hashes are in [`binaries.sha256`](2026-10-06-corrected-5848bce/build/binaries.sha256). |

Both payloads carry the version label 0.4.2, so tell them apart by package hash, not version.

### Installed behaviour

- **P04 storage-target suite.** `tests/privileged_vm/p04_targets_guest.py` ran unchanged except that it reads the package architecture from `dpkg`, on four empty 128 MiB virtual disks with fixed serials; no host or boot disk was attached. **35 of 35 checks passed** ([result](2026-10-06-corrected-5848bce/storage/p04-targets-result.json)), covering:
  - storage readiness, planning and protected target preparation
  - boot-disk exclusion, and refusal of mismatched filesystem types and serials
  - duplicate-UUID and replacement-disk detection, and refusal while swap is active
  - interruption and executor-death recovery without replaying an effect
  - all synthetic data, `fstab` and media paths surviving
- **Package identity.** After the suite, the installed binaries match the native build and the repository packages match the build; `dpkg --verify` reported no differences at all, not even in configuration files ([identity](2026-10-06-corrected-5848bce/storage/identity.json)).
- **Upgrades from genuine older payloads.** Each older package was hash-checked against its earlier evidence before installation; neither was rebuilt:

  | From | Earlier evidence | Result |
  |---|---|---|
  | 0.1.1 arm64 (`4ec15d2a…`) | P01 package checksums | Upgraded to 0.4.2 in 3.2 s; authority schema 1 → 6; user kept; session from 0.1.1 still valid (200); login and overview work; all three services active ([result](2026-10-06-corrected-5848bce/upgrade/from-0.1.1-result.json)) |
  | 0.3.2 arm64 (`76dbaf56…`) | P03 Compose ARM64 evidence | Upgraded to 0.4.2 in 3.1 s; schema 5 → 6; user and session kept ([result](2026-10-06-corrected-5848bce/upgrade/from-0.3.2-result.json)) |

  Both older packages were cross-built, so this proves the upgrade path, not native performance of those versions.

### Measurements

| Measurement | Measured | Budget | Within |
|---|---|---|---|
| `limeosctl status` (20 runs) | p50 1.39 ms, max 1.76 ms | under 20 ms | yes |
| `limeos-executor` start and exit (20 runs) | p50 1.60 ms, max 1.97 ms (`/bin/true` p50 0.66 ms for comparison) | under 20 ms | yes |
| Password worker start and exit (20 runs) | p50 2.06 ms, max 3.35 ms | under 20 ms | yes |
| `limeos-executor storage-inventory` (10 runs) | p50 52.5 ms, max 53.8 ms | none set; it includes raw probes of five block devices | — |
| Root storage reader (`limeos-storage-reader`, on demand) | starts in 8.8 ms; 1.13 MiB PSS, 3.38 MiB RSS; runs as root with zero effective and bounding capabilities, `NoNewPrivs`, `MemoryMax` 64 MiB | — | — |

The memory, CPU, write, startup and latency tables above were measured on the frozen payload and were **not repeated** for the corrected one. The correction changes three integer flag constants and adds no allocations, threads or I/O, so no difference is expected, but this report doesn't claim corrected-payload footprint figures.

## Branches

| Branch | Commit | Contents |
|---|---|---|
| `qual/arm64-native` | from `589cf9a` | This report, the evidence, `tests/qualification/arm64/`, and two one-line changes so the P04 guests read the package architecture from `dpkg`. No runtime code. |
| `qual/arm64-0.4.2-1087f24` | `5848bce` | `589cf9a` plus `git cherry-pick -x 1087f24`: the exact source of the corrected payload. |
| `fix/arm64-open-flags` | `08ca5b0` | My fix, written before `1087f24` appeared. Its runtime change is identical to `1087f24`, so it is superseded. Its one extra is a regression test for a symlinked `receipts.lock` in `limeos-executor-container`, which `1087f24` doesn't cover; it fails on frozen arm64 and passes with the fix ([log](2026-10-06-frozen-589cf9a/fix-verification/repro-frozen-with-new-tests.txt)). That test could be added on its own. |

## Harness revisions during the run

- **Identity check:** the first version in `install_guest.py` called `readelf`, which a clean guest lacks. It was replaced by an ELF-header read, and identity was rerun in the same installed frozen guest. Before the corrected-payload run, the check was also changed to report edited configuration files separately rather than failing on them; none were edited.
- **Stop step:** `native_vm.py`'s stop step was fixed after it left the first guest running (see Effect on wybie).
- **Cleanup after the runs:** `ruff format`, an explicit `check=False` (the default) on `subprocess.run` calls, `re.MULTILINE` for `re.M`, and executable bits. Behaviour is unchanged.

## Why this is not a Python or Pi comparison

These results sit beside the [reference Pi 5 baseline](../p00/2026-10-04-reference-pi5-baseline.md), but they can't be compared with it directly:

- **Kernel and paging.** The baseline ran on bare metal under the Pi kernel with 16 KiB pages. These services ran in a KVM guest with 4 KiB pages. RSS and PSS for the same binary are larger with 16 KiB pages.
- **Workload.** On wybie, the Python build serves real Jellyfin and *arr containers, the Mattermost assistant and five agent services. The guest ran busybox containers, had no assistant, and had only the 0.4.2 feature set.
- **Contention.** The guest shared the Pi's CPU with its live services, and the baseline didn't run under the same load. CPU and latency figures may carry some of that noise; memory figures do not.
- **Footprint.** The P00 footprint spike (4.55 MiB, ready in 23.5 ms) ran a minimal binary on bare metal. These figures are for the real installed services.

A comparable figure needs the native packages installed on the reference host or a spare Pi, measured in the same quiet window as a fresh Python baseline. That stays a separate gate.

## Not tested

- Bare-metal Pi measurement under the 16 KiB-page kernel, and any Pi 4 or SD-card host.
- Real media workloads, the assistant runtime (not in 0.4.2), and the P03 container-lifecycle suite with a real Engine on ARM64.
- 24-hour write soak and week-long `dpkg --verify` soak.
- `cargo deny` and `cargo audit` natively. Both are architecture-independent and covered by existing x86-64 runs.
- GitHub's native ARM64 CI, which is a separate claim per the brief.

## Effect on wybie

- **Packages:** `qemu-system-arm`, `qemu-efi-aarch64` and `qemu-utils` were installed with `--no-install-recommends`, pulling in 41 packages in total; nothing was removed or upgraded. At 00:36 the same 41 were purged, after an `apt-get -s` simulation confirmed the purge would remove exactly that set. Afterwards, wybie's package list (717 packages, same versions), running services and 18 containers matched the 23:24 snapshot, and no QEMU package or configuration remained. [Host record](2026-10-06-corrected-5848bce/host/host-impact.json).
- **Untouched:** nothing from LimeOS was installed on the host, no host service was restarted, and the Python checkout, its configuration and its containers weren't touched. All work lived under `~holly/limeos-arm64-qual/`.
- **Swap:** the 2.5 GiB build guest pushed about 190 MiB of wybie's own services into swap (315 MiB before, 503 MiB after, 511 MiB at cleanup, with 4.9 GiB of RAM available). Later guests used 1.5–2 GiB to avoid more of this. The swapped pages return on demand; no `swapoff` was forced on the host.
- **Leftover VM:** a stop-script defect left the first guest running for about 17 minutes after its work finished. The script now reads the root-owned PID file with `sudo` and fails loudly if the process is still alive.

## Effort

The work ran from 23:25 to 00:42 BST on 2026-10-05/06. Infrastructure waits inside that window: image download (2 minutes), apt and rustup in each build guest (about 2 minutes each), the 10-minute and 5-minute measurement windows, and guest boots (about 1 minute each). This is one overnight agent session, not a human effort estimate.
