# RW-005 result: Rust footprint on the reference Pi 5

Measured: 2026-10-04. **The RW-005 footprint gate passes.** Continue P00 with the pinned Rust toolchain and selected library stack. Provider and isolation spikes in RW-005, and the other P00 gates, remain open.

The experiment ran as `holly` on wybie, beside the existing Python services, from a private directory under `/tmp`. It bound to an ephemeral loopback port and read Docker inventory once per start. No service was installed, reconfigured, or restarted. The probe exited after measurement and its temporary databases were removed.

## Result

| Measurement | Pi 5 result | Acceptance |
|---|---:|---|
| Maximum idle PSS over 600 seconds | **4.55 MiB** | Below 15 MiB: pass |
| Maximum idle RSS | 5.89 MiB | Recorded alongside PSS |
| Maximum swap | **0 MiB** | Memory is not hidden in swap |
| Readiness, 10 starts | **15.8–23.5 ms**; p50 16.9 ms | Every start below 1 second: pass |
| Sequential cached overview, p95 | 0.38 ms | Experiment read; production latency remains unqualified |
| Three concurrent clients, cached overview p95 | 1.29 ms | Experiment read; production latency remains unqualified |
| CPU during 600 idle seconds | 0 ticks recorded at 100 Hz accounting | Below the provisional 0.1% of one-core target |
| Process disk writes during load / idle | 0 / 0 bytes | No unchanged-state commits during these windows |
| Process CPU during the load window | 0.21 CPU-seconds | 1,200 measured requests over 0.371 wall-seconds |
| ARM64 release executable | 4,674,088 bytes (4.46 MiB) | Binary size; full installed footprint is a later gate |

The measured process had 128 KiB of cumulative process disk writes after fresh database initialization. Those writes are included in startup work and excluded from the subsequent load/idle deltas. Zero recorded idle CPU ticks describes this accounting window, not a claim of literally zero CPU use.

## Workload and method

Reference hardware: Raspberry Pi 5 Model B Rev 1.0, four cores, Debian 12, kernel `6.12.25+rpt-rpi-2712`, NVMe OS disk. Its Docker inventory contained 18 containers, all running; each startup decoded the real 48,540-byte response.

The release binary initializes Axum, two Tokio workers, a retained Rustls HTTP client, and a dedicated SQLite worker. The database uses WAL and `synchronous=FULL`. The overview route serves cached container counts and performs a real SQLite read through a bounded 32-entry queue. The measured process has four threads: main, two Tokio workers, and SQLite.

Ten starts each used a fresh private database, including its initial durable writes. Time runs from subprocess creation to the first successful HTTP readiness response from the expected PID. OS page cache was warm; this is not a cold-cache or reboot test. Docker and database initialization must succeed before readiness.

The measured process then served 20 warm-up reads, 300 sequential reads, and 900 reads from three concurrent clients. Each read opened a new loopback HTTP/1 connection. After this load, the harness sampled `/proc/<pid>/smaps_rollup`, `stat`, and `io` once a second for 600 seconds. CPU is percent of one core; the harness is excluded. There are no idle Docker polls, model calls, or database commits.

## Build and verification

Rust 1.88.0; locked Axum 0.8.9, Tokio 1.53.2, reqwest 0.12.28, rusqlite 0.40.2, Rustls 0.23.45, and ring 0.17.14. SQLite is bundled. The Debian 12 compiler container produces both x86-64 and ARM64 release executables; the release profile uses optimization level 3, thin LTO, one codegen unit, stripped symbols, and abort-on-panic. The ARM64 binary's highest required glibc symbol version is 2.34, and it runs on wybie's glibc 2.36 without installing a toolchain there.

ARM64 binary SHA-256: `e252890d9e13cc2f482f6fb54d5ac6cba3071572f1ce3a75595be4dc0ff585d2`. The transferred binary and the result's binary identity match.

Validation passed: release builds for both architectures; Rust formatting and Clippy with warnings denied; Python formatting/lint; and three behavior tests on both the development machine and wybie. The behavior tests prove real Unix-socket HTTP and SQLite reads, no process disk writes from repeated GETs, rejection of POST, rejection of a public listener, and failed readiness when Docker is unavailable. They test the experiment and do not close production defects.

The first ARM64 compile exposed missing target C headers because the minimal cross-compiler install omitted recommended packages. The build recipe now explicitly installs `libc6-dev-arm64-cross`; the corrected locked build and native ARM64 execution pass. No application code change was needed for ARM64.

Raw evidence:

- [Pi 5 measurements, startup samples and 601 memory/CPU/I/O samples](2026-10-04-rw005-pi5-live.json).
- [Build provenance, compiler image digests, source hashes, dependency versions and commands](2026-10-04-rw005-build-provenance.json).
- [Both Debian binary checksums](2026-10-04-rw005-debian-binary-checksums.txt).
- [Python service and checkout snapshot before](2026-10-04-rw005-wybie-before.txt) and [after](2026-10-04-rw005-wybie-after.txt), byte-for-byte equal. All eight service PIDs and activation times are unchanged.
- [x86-64 fixture validation](2026-10-04-rw005-x86_64-fixture.json): 4.53 MiB PSS, 2.8–5.4 ms readiness, 30-second idle window. This used a fixture and intentionally did not qualify the Pi gate.

The local frozen source checkout still reports only the pre-existing untracked `Docs/LIMEOS_ROLLOUT_RECOVERY_TICKETS.md`. No files there were changed.

## What this establishes

The selected minimal Rust stack has enough measured headroom to proceed with the proposed footprint targets. This is a starting-cost measurement, not a feature-equivalent comparison with the complete Python application. It does not establish the combined 30 MiB core/executor budget, the 60 MiB configured-service budget, production authenticated latency, real dashboard behavior, full installation size, full-day write volume, or assistant/provider resource use. Rustls was initialized but no external TLS handshake was performed. Each remains a measured acceptance condition as its implementation arrives.

The compiler/library path is viable on both targets, and the Pi footprint spike passes without changing the accepted targets. Complete the inventory, remaining baseline measurements, threat model/ADRs, Linux/package/recovery mechanics, and provider/isolation spikes before P01. P00 remains in progress; all 50 production requirements remain open.
