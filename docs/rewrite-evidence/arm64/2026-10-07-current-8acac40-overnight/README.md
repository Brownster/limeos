# Native ARM64 overnight qualification: runtime 8acac40

Prepared overnight on 2026-10-06/07 on the reference Pi 5 (`holly@wybie`), under the operator's recorded window ([`authorization.json`](authorization.json): 21:30Z to 06:00Z). Brief: [Engineer 1: overnight native ARM64 qualification](../../../plans/2026-10-06-engineer-arm64-overnight-qualification.md), committed on main in `9fa8d71`. The authorization record cites it "at a595c2f", this branch's base, which predates the file. The record is left as written because each guest's `guest.json` binds its hash. Harness and reproduction steps: [`tests/qualification/arm64/`](../../../../tests/qualification/arm64/README.md). The integrator's recorder output is in [`../2026-10-07-current-8acac40-overnight-recorded/`](../2026-10-07-current-8acac40-overnight-recorded/qualification-status.json).

## Summary

Every required run completed, and the last guest stopped at 23:03:35Z, well before the 05:45Z teardown point. No runtime defect was found.

| Gate | Result |
|---|---|
| Native build: Rust 1.88.0 with locked dependencies, fmt, strict Clippy, workspace tests, contracts, dependency direction, release, standard and shadow packages, signed repository | **Pass** on the second attempt: 231 of 231 tests. The first attempt failed only the dependency-direction gate, because the build guest lacked `git`. That was a harness defect; the attempt is [retained](attempt-1-build-original-bundle/). |
| Fresh standard and shadow install, plus footprint workload | **20 of 20** checks |
| Storage suite on guarded synthetic disks | **50 of 50** |
| Container suite with a real Engine | **38 of 38** |
| Genuine schema 7 → 8 upgrade (0.4.3, CI arm64 artifact) | **Pass** |
| Genuine schema 6 → 8 upgrade (0.4.2, native `5848bce` artifact) | **Pass** |
| Optional storage services and command timings | Pass, except that `limeos-executor storage-inventory` p95 is 61.4 ms against the recorder's 20 ms ceiling ([below](#the-storage-inventory-budget-row)) |
| Native archive-inspector cases (optional) | 6 archives × 3 runs, all admitted; peak 4.3 MiB `VmHWM` |
| Deadline proof | The detached supervisor sent SIGTERM at the scheduled second, and the guest was gone within a second |
| Host teardown | 41 added packages purged; package list identical to the 717-entry snapshot; no QEMU, supervisor or staging files left |

**wybie's containers need the operator's attention.** At 22:18:53Z, a client of wybie's local Docker API stopped 14 running containers in sequence, flagging each as manually stopped. Jellyfin was started again at 22:19:32Z; the other 13 remain stopped. The evidence points away from this run ([below](#container-stop-on-wybie-at-221853z)). Nothing was restarted, because the brief keeps Docker workloads unchanged.

## Identities

| Item | Identity |
|---|---|
| Branch | `engineer/arm64-overnight-qualification` from `a595c2f1bcca8a8a0bc21ea0162eea841d0d5d15` |
| Runtime under test | `8acac403e32d3692b028a036a11f4fead2050580`; authority schema 8; package version `0.4.7+arm64.1` |
| Orchestrator | Guard and deadline supervisor `8e22c25`, committed at 21:41:07Z, before the first host command at 21:43:14Z. Fixes made during the run: `e7867f2`, `828860d` and `d46d7ee`. Inspector fixture: `ad3f9cd`. Post-run log fix: `84d6f81`. |
| Prepared bundle | `.cache/arm64-qual/2026-10-06-integration/source.tar.gz`: 1,688,191 bytes, SHA-256 `669675abab7b60c0a645ef70531dcd4890a144a681ee2b35c802af870069c594`, as the brief specifies. Used by build attempt 1, with fixtures at `8acac40`. |
| Rebuilt bundle | `79770ff579a9e0b36b733acc799af8f64da1f06faad94307d9789c168a2c5bd6`, 1,697,847 bytes, fixtures commit `d46d7ee`. It is not byte-identical to the prepared bundle. Its 557 runtime-source hashes and 3 frontend hashes equal the original manifest's; only five files under `tests/qualification/arm64/` differ. [Source manifest](build/source-manifest.json), SHA-256 `b0b98375…`. |
| Guest image | Debian 12 genericcloud arm64 (`Last-Modified` 2026-10-06 15:52:27 GMT). SHA-512 `835b929522716de9798d1c0fbe61caae8313ab78858b278f3389a2db922ae42e4619b4a776ecad39583c149fb85334a2f8cdae685bc56987160e5c86a2354375`, checked against Debian's `SHA512SUMS`. |
| Native binaries | `limeos-core` `aab46a60…`, `limeos-executor` `171bfb1e…`, `limeosctl` `a0ebf952…`, `limeos-password-worker` `17610bb6…`, all `AArch64`. Both native builds produced byte-identical binaries. |
| Packages | `limeos_0.4.7+arm64.1_arm64.deb` `6c86199b…`; `limeos-shadow_0.4.7+arm64.1_arm64.deb` `20e10c7e…`; signing key `24BC4738D97221AB2C15A847BBA79DBA0CAB439C`. Attempt 1's `.deb` files had different digests around the same binaries. They weren't kept, so the byte difference wasn't investigated. |
| Supervisor | `guest_supervisor.py` SHA-256 `29ada847…` for every guest in this run (recorded in each `guest.json`) |
| Upgrade inputs | Schema 7: 0.4.3, package `31e8eb3b…`, core `c41cb4ed…`, original source `f39396b`. Schema 6: 0.4.2, package `0a8651aa…`, core `15231244…`, original source `5848bce`. Each was re-signed into a disposable repository around unchanged bytes ([`sign-previous.sh`](sign-previous.sh)). |

## Hardware and method

| Item | Value |
|---|---|
| Hypervisor | Raspberry Pi 5 Model B Rev 1.0: 4 × Cortex-A76 (`0xd0b`), 8 GB RAM, NVMe. Host kernel 6.12.25+rpt-rpi-2712 with 16 KiB pages. |
| Virtualisation | QEMU 7.2.22 under KVM (`-accel kvm -cpu host`, `virt,gic-version=host`, AAVMF 2022.11), at `nice 19` with `ionice -c 3`. QEMU opened `/dev/kvm` as root, then dropped to `holly` with `-runas`. Guests used user-mode networking only, reached through `ProxyJump` to a loopback port, with no host directory or block-device passthrough. |
| Guest | Debian 12, kernel 6.1.0-53-cloud-arm64 with 4 KiB pages. `systemd-detect-virt` reports `kvm`, and the CPU reports implementer `0x41`, part `0xd0b`. |
| Guest sizes | One guest at a time. Builds and inspector: 3 vCPU, 2048 MiB, 24 GiB or 16 GiB overlay. Tests: 2 vCPU, 1536 MiB, 16 GiB overlay. The storage guest also had four empty 128 MiB qcow2 disks with fixed serials. Deadline proof: 1 vCPU, 512 MiB. |
| Frontend | Plain JavaScript, built and hashed on the workstation from the frozen commit. All Rust was compiled natively in the guest. |

## Results

### Build

The first attempt used the prepared bundle. Every step passed except `dependency-direction`, which raised `FileNotFoundError` because `scripts/check_repository.py` calls `git` and the build guest hadn't installed it ([log](attempt-1-build-original-bundle/logs/dependency-direction.txt)). `build_guest.py` rightly accepts no build with a failed gate. In the same guest, the check passed once `git` was installed ([transcript](attempt-1-build-original-bundle/supplementary-check-repository-with-git.txt)). The fixture now installs `git` (`828860d`). Because the guest fixtures changed, the bundle was rebuilt with a new fixtures commit and the same runtime source.

The second attempt passed every gate: fmt, Clippy with `-D warnings`, **231 tests passed, 0 failed, 0 ignored**, contracts, dependency direction, release, both packages and the signed repository. [Result](build/build-result.json), [logs](build/logs/), [artifact hashes](build/artifacts.sha256).

### Install and footprint

All 20 checks passed ([result](install/install-result.json), [console](install/install-console.txt), [journal](install/journal-limeos.txt)):
- **Platform and identity:** native `aarch64` under KVM; signed install of `0.4.7+arm64.1`; `dpkg --verify` reports no mismatches; installed hashes match the native build.
- **Units and permissions:** core and both executors enabled and running; the storage reader, readiness and targets units `static` and inactive. All 13 root-executed paths are root-owned and not writable by other accounts. Only the container executor's process has the Docker group.
- **Passwords and sessions:**
  - Imported Werkzeug `scrypt:32768:8:1` and `pbkdf2:sha256:1000000` hashes verify and are upgraded to Argon2id.
  - The password worker runs bounded and short-lived: at most one at a time, with 6 workers for 10 logins (4 answered 429). Peak worker usage was 19.5 MiB PSS (20,010 KiB; 21,188 KiB `VmHWM`), with none left running at idle.
  - Sessions survive restarts of core and both executors.
- **Shadow install:** installs beside standard, keeps its state separate, and refuses start, stop and restart ceilings.

Measurements as compared by the recorder ([`budgets.json`](../2026-10-07-current-8acac40-overnight-recorded/budgets.json)):

| Measurement | Measured | Budget | Within |
|---|---|---|---|
| Core and base executors, idle PSS | 10.44 MiB (core 7,853 KiB, container executor 1,549 KiB, storage executor 1,293 KiB) | 30 MiB | yes |
| All measured resident LimeOS services, idle PSS (no assistant in this payload) | 10.44 MiB | 60 MiB | yes |
| Steady-state swap | 0 KiB | 0 | yes |
| Core and executors after the workload and shadow removal | 10.87 MiB | 30 MiB | yes |
| Idle CPU over 600 s | 0.051% of one core | 0.1% | yes |
| Idle writes (process `write_bytes` over 600 s, extrapolated) | 4.72 MB/day | 20 MB/day | yes |
| CPU with 3 open dashboard streams over 300 s | 0.088% of one core | none set | — |
| Writes with 3 dashboard streams (300 s, extrapolated) | 4.72 MB/day | 20 MB/day | yes |
| Core restart to ready, 20 restarts | p50 250.7 ms, p95 266.1 ms, max 268.0 ms | 1 s | yes |
| Full stack cold start to ready, 5 starts | max 456.6 ms | 1 s | yes |
| Cached overview, sequential, p95 | 0.92 ms | 20 ms | yes |
| Authenticated reads, 3 concurrent clients × 200 requests, p95 | 3.52 ms | 20 ms | yes |
| Installed package bytes, including the UI | 11.15 MB | 154 MB | yes |
| `limeosctl status`, 20 runs | p50 1.47 ms, p95 1.71 ms | 20 ms | yes |
| `limeos-executor storage-inventory`, 10 runs | p50 57.3 ms, p95 61.4 ms | 20 ms | **no** |

These figures are reported separately and are not counted against LimeOS:
- **Shadow services:** 8.27 MiB PSS (core 5,732 KiB, container executor 1,414 KiB, storage executor 1,318 KiB).
- **Container stack:** Docker Engine 78.7 MiB PSS, containerd 49.6 MiB, 20 `containerd-shim` processes 149.6 MiB.
- **Workload:** 20 static busybox containers, 2.2 MiB in total.

### Optional storage services

These were measured in the storage guest after its suite ([result](storage/extra-result.json), [console](storage/extra-console.txt)):

| Service | Start | Memory | Privilege |
|---|---|---|---|
| `limeos-storage-reader` (on demand) | 44.1 ms | 838 KiB PSS, 3,064 KiB RSS | Root UID, with zero effective, bounding and ambient capabilities, `NoNewPrivs`, `MemoryMax` 64 MiB |
| `limeos-storage-targets` (standard only, dormant until configured) | 37.7 ms | 965 KiB PSS, 3,184 KiB RSS | Root UID, with `CAP_CHOWN` as its only effective, bounding and ambient capability, `NoNewPrivs`, `MemoryMax` 64 MiB |

#### The storage-inventory budget row

The architecture's start-up row sets "executor tasks and `limeosctl` under 20 ms". `record_run.py` applies that ceiling to `limeos-executor storage-inventory`, so the recorded result is **not within budget**. Most of the measured interval isn't process start-up. The command runs `lsblk`, then `blkid -p` against a retained descriptor for each block device. The storage guest had a system disk, the cloud-init seed and four synthetic disks. The previous native run measured the same command at p50 52.5 ms and listed no budget for it. Nothing was relaxed. Whether this ceiling is meant to include raw device probing is a decision for the integrator. For comparison, `limeosctl status` runs at 1.5 ms.

### Storage suite

All 50 checks passed on the guarded synthetic disks ([result](storage/storage-result.json), [console](storage/storage-console.txt), [journal](storage/journal-limeos.txt)):
- **Disk guards:** the serial, size, boot-disk and mount checks ran before any formatting. Type, serial and alias mismatches were refused, as were root-filesystem aliases and duplicate raw UUIDs.
- **Target preparation:** creates only empty root-owned directories.
- **Interruptions:** real executor and core SIGKILLs, before and after the effect, keep complete claim sets. Receipt lookup and lost-response replay cannot repeat an effect, and explicit reconciliation releases claims only when the outcome is proved.
- **Packages:** a real `apt` replacement and removal of standard and shadow. Shadow removal keeps the standard target service, and standard `prerm` stops every optional root service before binaries are replaced.

The suite reports its own limits: mount and fstab effects, live dependency discovery and unmount handling remain pending.

### Container suite

All 38 checks passed against Debian's `docker.io` 20.10.24 with a static busybox fixture ([result](container/container-result.json), [console](container/container-console.txt), [journal](container/journal-limeos.txt)):
- **Lifecycle:** real restart, start and stop, each approved, independently verified, bound to its action and idempotent.
- **Interruptions:** both core and the container executor killed before prepare, during and after the effect, for every operation, with no unjustified second effect.
- **Approvals and grants:** expired approvals and queued cancellations cause no Engine effect.
- **Logs:** bounded, filtered and with no effect.
- **Ceilings:** each effect has its own independent ceiling.
- **Shadow:** isolated, and refuses write-enabled ceilings.

The suite's footprint after the workload was 10,731 KiB (10.48 MiB) PSS, with no swap.

### Genuine upgrades

Each upgrade ran in its own fresh guest, in the brief's priority order ([schema 7](upgrade-schema7/upgrade-result.json), [schema 6](upgrade-schema6/upgrade-result.json)):

| From | Old package and core | Preserved across the upgrade | After the upgrade |
|---|---|---|---|
| 0.4.3, schema 7 | `31e8eb3b…`, core `c41cb4ed…` | Human session; approved canonical bytes and token hash; verified receipt; four active complete claim sets; queued state | Approved restart verified; replaying the original queue response returns the same job |
| 0.4.2, schema 6 | `0a8651aa…`, core `15231244…` | Human session; approved canonical bytes and token hash; verified receipt; four active primary locks; queued state | Approved restart verified; replaying the original queue response returns the same job |

Both old packages are the original bytes ([schema 7 provenance](upgrade-schema7/previous-artifact.json), [schema 6 provenance](upgrade-schema6/previous-artifact.json)). Only the disposable repository signature is new. The CI `0.4.4`/`0.4.4+ci.1` labels were not used.

### Native archive inspector (optional)

The frozen `limeos-backup-archive` `inspect` example was built natively with `cargo build --release --locked`: 660,088 bytes, SHA-256 `16e0509e…`. It was run three times per archive under the frozen [`measurement-policy.json`](../../p04/rw043/measurement-policy.json) (SHA-256 `03069532…`, zstd window 2^21). [Result](inspector/inspector-result.json).

| Archive | Compressed bytes | Peak `VmHWM` | Time |
|---|---|---|---|
| `legacy-valid.tar.zst` (fixture) | 786 | 2,084 KiB | 2–3 ms |
| `legacy-valid.tar.gz` (fixture) | 834 | 912 KiB | 2 ms |
| `zeros-64m.tar.zst` (fixture) | 2,214 | 4,172 KiB | 0.35 s |
| 64 MiB of zeros, zstd (generated) | 2,204 | 4,176 KiB | 0.35 s |
| 64 MiB of random data, zstd (generated) | 67,110,937 | 4,372 KiB | 0.75 s |
| 64 MiB of random data, gzip (generated) | 67,119,582 | 2,064 KiB | 0.76 s |

All were admitted with exit 0. Memory stays flat as archives grow: about 2 MiB plus the 2 MiB zstd window. This is the standalone example's own peak RSS. It isn't installed-service PSS or a restore measurement. It is also lower than the x86-64 workstation figures in the RW-043 evidence.

## Deadline and shutdown evidence

**Guard.** The commit `8e22c25` came before any host command:
- `native_vm.py` refuses wybie unless `--authorization` names a record with exactly the expected keys, the exact `holly@wybie` target, and a UTC window of at most 12 hours that contains the current time.
- The record is re-checked before every SSH, `scp` and host command.
- Production hosts are matched by hostname label, by `ssh -G` resolution (which makes no connection) and by resolved address, so another alias or IP can't bypass the guard.

Local regressions cover default denial, admission within the named window, and denial for expiry, mismatch and malformed records (`AuthorizationTests`, 5 tests).

**Supervisor.** Each guest has its own detached `guest_supervisor.py` on wybie, started with `setsid nohup`, so it outlives the SSH session that launched it.
- **Binding:** it binds the guest's QEMU by `pidfd`, kernel start time, UID and the guest's unique `-name` marker, and it signals only through that pidfd.
- **Deadlines:** SIGTERM at the deadline, then SIGKILL two minutes later; inside this window that meant 05:55:00Z and 05:57:00Z.
- **Tests:** real local processes show that only the bound process is forced, an unrelated process survives, a mismatched identity is never signalled, and supervision ends when the guest stops first (`SupervisorTests`, 4 tests). All 18 harness tests pass.

**Deadline proof.** A disposable guest booted at 21:44:27Z with `--terminate-at 21:47:21Z` ([supervisor log](deadline-proof/supervisor.log), [guest record](deadline-proof/guest.json)):
- The launching SSH session had long since exited when the supervisor sent SIGTERM.
- The guest exited, and the supervisor logged `guest_gone` at 21:47:21Z. SIGKILL wasn't needed.
- The host then showed no QEMU process.

The SIGTERM line reads 21:47:20Z, but the signal wasn't early. The supervisor decides from `time.time()`, while it stamped log lines with `time.gmtime()`, which reads the coarse kernel clock. Just after a second boundary, the coarse clock still shows the previous second; that happened at five of five boundaries tested on the workstation. `84d6f81` now stamps events from the precise clock, to the millisecond, and the regression asserts that no signal is logged before its deadline. All guests in this run used the earlier supervisor.

**Every guest.** Each supervisor bound at boot and was released by an orderly stop before any deadline:

| Guest | Bound | SIGTERM / SIGKILL deadline | Ended |
|---|---|---|---|
| Deadline proof | 21:44:27Z | 21:47:21Z / 21:49:21Z | 21:47:21Z, after SIGTERM |
| Build attempt 1 | 21:49:03Z | 05:55:00Z / 05:57:00Z | 22:01:39Z, orderly stop |
| Build | 22:02:14Z | 05:55:00Z / 05:57:00Z | 22:13:52Z, orderly stop |
| Install and footprint | 22:14:13Z | 05:55:00Z / 05:57:00Z | 22:35:31Z, orderly stop |
| Storage | 22:35:54Z | 05:55:00Z / 05:57:00Z | 22:39:53Z, orderly stop |
| Container | 22:40:20Z | 05:55:00Z / 05:57:00Z | 22:48:03Z, orderly stop |
| Upgrade from schema 7 | 22:48:10Z | 05:55:00Z / 05:57:00Z | 22:52:15Z, orderly stop |
| Upgrade from schema 6 | 22:52:21Z | 05:55:00Z / 05:57:00Z | 22:55:57Z, orderly stop |
| Inspector | 23:00:26Z | 05:55:00Z / 05:57:00Z | 23:03:35Z, orderly stop |

**Teardown**, recorded through the guarded `host-exec` ([command log](host-commands.jsonl)):
1. **Simulation:** at 23:06:43Z, `apt-get -s purge` of the [41 added packages](host-packages-added.txt) listed exactly that set and nothing else ([simulation](host-purge-simulation.txt)).
2. **Purge:** the real purge followed at 23:07:08Z.
3. **Staging removed:** at 23:07:21Z, `~/limeos-arm64-qual`, which held only the 327 MiB image, `SHA512SUMS` and the download headers.
4. **Package list:** the [list after teardown](host-packages-after-teardown.txt) is byte-identical to the [717-entry snapshot](host-packages-before.txt) taken before installation.
5. **Final check:** at 23:07:31Z there was no `qemu` or `guest_supervisor` process, and no `limeos` path in `~holly`. `holly`'s groups were recorded; no account or group change was made. `pi-health`, `pihealth-helper` and `docker` were active.

## Harness defects found and fixed

Each fix is its own commit, and none changes runtime source or relaxes an assertion:
- **`e7867f2`: stop after a supervisor kill.** QEMU deletes its pidfile when it exits, so `stop` found no PID after the deadline proof. Boot now records `qemu.bound-pid`.
- **`828860d`: build guest without `git`.** This failed the dependency-direction gate in build attempt 1, as described [above](#build).
- **`d46d7ee`: stop of a running guest.** `stop` matched QEMU by working directory, but a daemonized QEMU runs in `/`, so the attempt-1 guest wasn't stopped at first; its supervisor stayed bound. `stop` now matches the guest's own `-name` marker in `/proc/PID/cmdline`, and the guest stopped at 22:01:39Z.
- **`84d6f81`: supervisor log stamps.** Found after the run, as described [above](#deadline-and-shutdown-evidence).

`record_run.py` refuses to write into an existing run directory, so its output is in the sibling [`-recorded`](../2026-10-07-current-8acac40-overnight-recorded/) directory. Its `evidence-sha256.json` hashes every recorded artifact. The two upgrade results were staged under distinct file names, because the recorder rejects duplicate artifact names.

## Effect on wybie

- **Packages.** With the operator's approval ([decision](host-package-decision.json)), `qemu-system-arm`, `qemu-efi-aarch64` and `qemu-utils` were installed with `--no-install-recommends` at 21:43Z. That added 41 packages; no existing package was removed, upgraded or downgraded. No `apt-get update` ran. The install reused `.deb` files already in `/var/cache/apt/archives`, apparently left by the previous night's run, and no file there changed during this window. All 41 packages were purged at 23:07Z.
- **Untouched.** Nothing from LimeOS was installed on the host. No host service, mount, account or group was changed, and nothing was restarted. The Python checkout, its configuration and its databases were only read, through the journal and logs, while investigating the container stop.
- **Memory pressure.** Even within the 2048 MiB ceiling, the build guests pushed some of wybie's own pages into swap: from 0 MiB at preflight, to 117 MiB after build attempt 1, to 209 MiB after build 2. `MemAvailable` reached its lowest point, 3.9 GiB, just after each build. Swap fell to 88 MiB once the containers stopped at 22:19Z and stayed there. Load averages ranged from 0.00 to 1.98. A future build on this host should use 1536 MiB or accept some swap.

### Container stop on wybie at 22:18:53Z

What happened, from Docker's own events and journal ([logged queries](host-commands.jsonl)):
- At 22:15:14Z, the host sample showed 14 containers up for about 2 hours.
- From 22:18:53Z to 22:19:16Z, Docker sent SIGTERM to each of the 14 in turn, in exactly the order `docker ps` lists them: `sonarr`, `transmission`, `jackett`, `radarr`, `rdtclient`, `sabnzbd`, `vpn`, `audiobookshelf`, `get_iplayer`, `navidrome`, `jellyfin`, `yt-to-jellyfin`, `requestrr` and `watchtower`.
- `yt-to-jellyfin` was killed at 22:19:24Z, after the 10-second stop timeout. `dockerd` logged `hasBeenManuallyStopped=true` for all 14, so their restart policies didn't bring them back.
- There were no `create`, `destroy` or `pull` events, so it wasn't an image update.
- Memory wasn't short: `MemAvailable` was 5.2 GiB at 22:15Z.
- `jellyfin` was started at 22:19:32Z. At 23:07Z it was the only one of the 14 running.

Why this run is an unlikely cause:
- `dockerd` runs with `-H fd://`, with no TCP listener and no `daemon.json`, so only a local Unix-socket client could make these requests.
- The harness ran no host command between 22:01:41Z and 22:56:00Z. Its only Docker calls on the host are the read-only `docker ps` in its host samples, at 22:15:14Z and 22:20:21Z.
- The install guest running at the time had its own Docker with 20 busybox containers, none with these names. It had no route to the host's socket.
- Two workstation SSH sessions opened during that minute, as `ProxyJump` forwards to the guest. `ProxyJump` uses `-W` forwarding, which runs no command on the host.

What the evidence doesn't show: sshd at its default log level doesn't record channel types, and the `pi-health` and `pihealth-helper` journals have no entries for 22:10–22:25Z. So the client that stopped the containers isn't identified. The order matches iterating over `docker ps`, as a "stop all" action or `docker stop $(docker ps -q)` would; Jellyfin being started again 23 seconds later suggests someone was using it.

**Recommendation:** ask whether anyone stopped the stacks around 23:19 BST. If not, start them deliberately and check `watchtower`, which exited with status 1.

Already down before the window: four other containers (`airsonic-advanced`, `limeos-alertd`, `limeos-mattermost` and `limeos-mattermost-db`) had exited about three hours before 23:07Z. wybie last booted at about 20:23Z. The Python assistant (`limeos-agent`) logs `listener transport failed` every minute, consistent with Mattermost being down. This run didn't cause or change any of it.

## Not tested

- **Hardware:** bare-metal measurement under wybie's 16 KiB-page kernel, and any Pi 4 or SD-card host. These are KVM-guest figures with 4 KiB pages.
- **Python comparison:** a comparison with the Python build under a matched workload. Here the guest ran busybox containers with no assistant, while the host's Python build serves real media containers and agent services. The two shared the CPU without a matched quiet window. This stays a separate gate.
- **Soaks:** the 24-hour write soak and the week-long `dpkg --verify` soak. The write figures extrapolate 600-second and 300-second intervals.
- **Workload coverage:** the footprint with eight registered storage disks, a real media workload, and an assistant runtime, which isn't in this payload.
- **Storage effects:** mount, fstab, unmount and loss effects, and live container-storage dependency discovery. These are pending in the runtime itself, as the suites report.
- **Supply-chain checks:** native `cargo deny` and `cargo audit`. Both are architecture-independent and covered by existing runs.
- **CI:** GitHub's native ARM64 CI, which is a separate claim.

## Effort

| Item | Value |
|---|---|
| Elapsed | One agent session, 21:34Z to about 23:30Z on 2026-10-06. That's elapsed agent time, not human engineering hours, so it doesn't revise the 24-hour estimate. |
| Guest wall time | About 79 minutes from the first boot (21:44Z) to the last stop (23:03Z), including two native builds of about 11 minutes each, the 20-minute install and footprint run, and the 600-second and 300-second measurement windows |
| Infrastructure waits | Image download (under 1 minute); `apt` and `rustup` in each build guest (about 1.5 minutes); guest boots (about 1 minute each) |
| Rework | One extra native build (about 12 minutes), caused by the missing `git` |
