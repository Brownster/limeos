# RW-005: Rust footprint experiment

Prove that the planned libraries fit on the reference Pi 5 before building production services. The initial gate is idle PSS below 15 MiB and every measured start ready in under one second, with no swap hiding memory use. Ten starts and a ten-minute idle observation qualify the recorded result. The [2026-10-04 Pi 5 result](../../docs/rewrite-evidence/p00/2026-10-04-rw005-footprint-result.md) passes.

The probe runs two Tokio workers and one SQLite worker. It initializes a Rustls HTTP client, performs exactly one bounded `GET /containers/json?all=1` through the Docker Unix socket, creates a fresh SQLite database in its private temporary directory with WAL and `synchronous=FULL`, and serves cached container counts plus a real database read through Axum. Readiness follows Docker and database initialization. Docker failure fails startup; it cannot produce a passing empty observation.

This is an experiment, not an authenticated production core or an executor. It binds only to loopback on an ephemeral port. It has no mutation routes, schedules, user database, provider login, or outside HTTPS request. The Docker socket is accessed directly for this one read; P01/P03 must establish the production executor boundary. The Python application, its state, systemd units, and port 8002 are untouched.

## Build and check locally

Rust 1.88.0 is pinned. `Cargo.lock` fixes dependencies. The release profile uses optimization, thin LTO, one codegen unit, symbol stripping, and abort-on-panic. SQLite is bundled; Rustls uses the dependency's selected crypto backend, without OpenSSL.

```bash
cargo build --locked --release -p limeos-footprint
cargo fmt --all --check
cargo clippy --locked --all-targets -- -D warnings
uv run --no-project spikes/footprint/test_probe.py
uv run --no-project spikes/footprint/measure.py \
  --binary target/release/limeos-footprint \
  --fixture --idle-seconds 30 --output /tmp/limeos-local.json
```

The fixture exercises the Unix-socket HTTP client with 18 running containers and one stopped container. It never qualifies the reference-host gate, even if its memory and startup pass.

## Build for Debian 12, x86-64 and ARM64

The compiler container uses Debian 12, avoiding an executable linked against the development machine's newer glibc. It is a build tool, not a LimeOS distribution image. No Rust toolchain is installed on wybie.

```bash
docker build -f spikes/footprint/Containerfile -t limeos-footprint:rw005 .
docker create --name limeos-rw005-artifacts limeos-footprint:rw005
mkdir -p target/rw005
docker cp limeos-rw005-artifacts:/artifacts/. target/rw005/
```

The image produces both release binaries, compiler identification, and SHA-256 checksums. Review `ldd` and the ELF target before running the ARM64 binary on the Pi. Preserve the lockfile, image digest, binary checksum, and raw result with the report.

## Measure on wybie

Wybie is a production media server. Use the existing passing result; repeat an active measurement only in an operator-agreed quiet window with no playback. Validate changes locally with the fixture and on the separate test host first. This harness includes a concurrent request burst, so it is not a passive observation during streaming.

Copy the ARM64 binary and `measure.py` into a new private directory under `/tmp` on wybie. Run as `holly`, using its existing permission for the single Docker read. Use the system Python 3.11 to run the dependency-free measurement harness; this does not import or develop the frozen Python application.

```bash
python3 measure.py --binary ./limeos-footprint --output result.json
```

The harness starts and stops only its own probe processes. Every start uses a fresh temporary database and an ephemeral loopback port. It removes its private probe state after measurement. Copy the result back into `docs/rewrite-evidence/p00/`; retain the uploaded measurement bundle until the evidence has been verified.

The result records:

- Ten launch-to-HTTP-readiness times, including Docker and database initialization. OS page cache is warm; these are not reboot or cold-cache measurements.
- Cached overview latency, sequentially and with three concurrent clients, using a new HTTP connection per request. These unauthenticated experiment reads do not establish production endpoint parity or authenticated latency.
- PSS, RSS, swap, thread count, process CPU ticks, and process disk-write counters sampled once a second over ten idle minutes after the load test.
- Hardware model, architecture, kernel, OS, container counts, binary size and hash, workload settings, and explicit gate results.

CPU is percent of one core. The harness process is excluded. Memory is the probe's measured resident PSS, not a count of binary bytes or reserved virtual memory. Disk writes cover this process; they do not prove a full-day write budget for all future services. Idle observations serve cached data and do not poll Docker or commit unchanged state.

The harness exits successfully when it captures a valid result. Enforce the reference-host gate in a batch run with `jq -e '.gates.rw005_footprint_pass' result.json`; a fixture or short run cannot pass it.

Passing this spike supports the language/library choice. The combined 30 MiB core/executor target, 60 MiB configured-service target, production latency, provider memory, and full-system resource use remain later acceptance work.
