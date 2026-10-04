# P01 execution tracker and acceptance record

Implementation delivered 2026-10-04, with the operator's permission to proceed from the positive P00 footprint result. P01's local gate is passed for the health-only foundation. Later phase effects and cutover remain gated by their own evidence. The Python reference and wybie were not changed.

Estimate: eight implementer days (64 hours), with review at 96 hours. Observed implementation/evidence window: 2026-10-04 19:09–21:15 UTC, approximately 2.11 hours of wall-clock time including automated builds/tests. This is one agent session, not a measured human-effort figure.

## Delivered work

| Package | Result |
|---|---|
| RW-010 | Rust 1.88 workspace; exact dependencies; committed Cargo.lock; dependency direction and unsafe gates; generated protocol/schema/OpenAPI/TypeScript; native x86-64/ARM64 CI configuration |
| RW-011 | Local one-use bootstrap; no default identity; bounded Werkzeug scrypt/PBKDF2 verification with atomic Argon2id upgrade; durable sessions; role/resource grants and revisions; CSRF/origin/rate limits; independent executor ceilings |
| RW-012 | Single-instance database worker; WAL/FULL schema v1; transactional intent/events; idempotency/resource locks; task UID/scope/revision/generation/expiry; conservative restart reconciliation; atomic configuration; space/audit refusal; worst-case budget reservations and unique settlement |
| RW-013 | x86-64/ARM64 Debian packages; signed test apt repository; root-owned payload; dedicated identities/sockets; tested systemd limits; checked, repeatable maintainer scripts; local status/enrollment commands |

Executor protocols permit health only. P01 contains no host/container mutation or provider call. Receipt-driven reconciliation, plans/approvals, provider connections and the full migration workflow arrive with their later phase consumers.

## Tested build

- Implementation snapshot: `d98e4726a8791dfe4152e68969323fc4826185aa`; source digest `49f45a0e658dd173836f9e2f52216068d9753ce42836fa61939932d1a484609a`. See [build provenance](build-provenance.json) for per-file, per-binary and package SHA256 values
- Debian fixtures: 0.1.0/0.1.1 for amd64 and arm64; both versions intentionally use authority schema 1. Upgrade/downgrade evidence does not promise future schema rollback
- Native release build: x86-64 Debian 12 with Rust 1.88.0. ARM64: Debian cross sysroot plus a successful QEMU-user CLI smoke test. Native ARM64 execution remains to be qualified
- Signing fingerprint `5046FB0972B8DF0DCD7A225B54D35B8B21AE1BF1` is a disposable test key; `gpgv` verified InRelease. No production key or repository was published
- Local Git was initialized so lockfiles and source are committed. No remote is configured and nothing was pushed

## Passing commands and evidence

Executed on 2026-10-04 against that source and those package fixtures:

| Command | Result and evidence |
|---|---|
| `cargo fmt --all -- --check` | Pass |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | Pass |
| `cargo test --workspace --locked` | 23 passing Rust test targets/scenarios, including the crash-child entry point; [raw test output](rust-tests.txt) |
| `cargo deny check` | Advisory/license/source/wildcard checks pass; duplicate-version warnings only; [output](cargo-deny.txt) |
| `cargo audit --json` | Zero vulnerabilities or informational warnings across 211 dependencies; [JSON](cargo-audit.json) |
| `shellcheck packaging/debian/postinst packaging/debian/prerm packaging/debian/postrm packaging/repository.sh` | Pass |
| `systemd-analyze verify packaging/systemd/*.service` | Pass with fixed executable paths present |
| `python3 scripts/check_repository.py` | Dependency direction and tracked-output hygiene pass in the initialized Git repository |
| `python3 scripts/check_contracts.py` | Generated contracts match Rust |
| `npm ci --ignore-scripts`, `npm test`, `npm run build`, `npm audit --audit-level=high` in frontend | Three passing client tests, strict type check/build pass, zero vulnerabilities; [test output](frontend-tests.txt) |
| `uv run scripts/test_gates.py --container limeos-p01-build` | Thirteen deliberate violations rejected; [gate probes](gate-probes.json). CI invokes the same probes natively |
| `cargo build --locked --release -p limeos-core -p limeos-executor -p limeosctl`, plus the ARM64 target with cross CC | Both release architectures built, including the password worker |
| `uv run packaging/build.py --arch ARCH --binaries RELEASE_PATH [--version 0.1.1]` | Four root-owned packages built; [package checksums](package-checksums.txt) |
| `packaging/repository.sh /build/dist/test-repo TEST_FINGERPRINT dist/*.deb` in the isolated builder, followed by `gpgv` | Signed apt metadata verifies; architecture indices contain only their matching architecture |
| `uv run tests/privileged_vm/run.py --repository dist/test-repo` | Fourteen clean Debian 12 KVM acceptance groups pass; [VM result](vm-result.json) |
| `uv run --with ruff ruff check packaging/build.py scripts tests/privileged_vm` | Development harness lint passes |

The VM booted a checksum-verified Debian cloud image with two vCPUs, 1 GiB RAM and kernel 6.1.0-53-cloud-amd64. It tested root/sudo reruns, rejection of unsafe existing accounts, enrollment, cookie/origin/CSRF behavior, kernel peer UID checks, denied socket/Docker access, root executable/library permissions, malformed and oversized traffic, slow headers, preserved corrupt configuration/ceilings, stricter ceiling preservation, service failure injection, upgrade/downgrade, package tamper detection, actual disk pressure, idle reads, log secret exclusion and remove/reinstall recovery. The harness destroyed its overlay, SSH key and VM after completion.

## Footprint and decisions

After authentication, core plus both resident executors used **5.19 MiB PSS**, with no swap. Five full core restarts reached RPC readiness in **93.5–132.3 ms**. Ten CLI status calls took **1.12–1.93 ms**. Per-service PSS/RSS and cgroup peak/swap counters are in the VM JSON. These are VM results, not new Pi measurements.

The first candidate passed functional acceptance but retained 43.31 MiB combined PSS after in-process password work, failing the 30 MiB target. [That result is preserved](vm-before-password-worker.json). The final implementation runs password work in bounded short-lived Rust processes, then releases that memory. [ADR 0001](../../adr/0001-p01-foundation.md) records the decision. Debug crypto builds are optimized so tests keep the production deadline rather than weakening it. A FIFO configuration probe initially timed out; the regular-file/nonblocking guard now rejects it immediately. The rapid-restart benchmark also hit systemd's intended five-starts/ten-seconds limit; [diagnostics](vm-failure.txt) are retained. Benchmark counter resets do not remove the production rate limit.

The ownership matrix and operator recovery steps are in [P01 operations](../../p01-operations.md). The [dependency/native-code boundary audit](dependency-boundary-audit.md) records transitive unsafe/FFI and advisory scope.

## Defect closure and remaining qualification

Nine rows close on executable P01 regression evidence: SEC-001, LIVE-001, INST-001/002/003, LIVE-004, DEP-001, PLG-002 and ARCH-001. Shared later-phase rows remain open. CI-001 has thirteen local rejection proofs and a wired workflow; remote execution is not yet observed. PKG-001's package-only workflow is present; full release publication is not exercised. HYG-001 remains open for the companion-app decision. CFG-001's week-long package verification and LIVE-003's 24-hour write target require soak evidence.

Still required before the relevant production use/cutover: native ARM64 CI and Pi 4/5 workload qualification in an agreed quiet window; actual browser/HTTPS proxy acceptance; long-running write/package-integrity soak; production signing/repository operation; all unresolved P00 provider, storage, mount-namespace and recovery assumptions; and the later phase migration/effect scenarios. No live mutation or benchmark ran on wybie during P01.
