# Engineer delivery integration

The integrated runtime and fixtures are frozen at `8acac403e32d3692b028a036a11f4fead2050580`. Both deliveries started at `60e6309384c6a93caa63c0d578dc57d981897863`. Original engineer branches and historical evidence are preserved; cherry-picks retain their authors. `commit-map.json` records the mapping. The inspector formatting follow-up was folded into its implementation commit.

## Reviewed changes

- **RW-043:** closed pure admission policy, bounded gzip/zstd inspection, deterministic digest-bound manifests and adversarial tests. The integration fixes short/interrupted reads and coalesces only zero-length self-links after the exact regular file has been fully inspected. Manifest v2 records these inert repeats; no link or additional destination is created. Other links, duplicate files, unsafe paths and unmapped originals remain rejected.
- **Repository gate:** every crate manifest needs an explicit dependency-boundary registration. A deliberate unregistered crate now fails the gate.
- **ARM64 harness:** schema 8 bundle/build/package/installed-hash binding, genuine historical upgrade provenance, crash suites and footprint collection. Nine local harness regressions pass. The runner refuses wybie by name before SSH; only an explicitly supplied isolated host is eligible.

The [archive design](../../../p04-backup-archive-admission.md) defines the future restore executor's requirements. **BKP-001 remains open.** Production limits/mappings require the P00 inventory, and restore effects, database-aware recovery, backup creation and scheduling are still pending. Current installed native ARM64 and comparable Python performance are untested.

## Local verification

`checks.json` records commands and scope. The workspace transcript shows **231 passed, zero failed**; the inspector contributes 3 unit and 19 integration tests, and the domain contributes 10 backup tests. Two transcripts have only extra EOF blank lines removed; the raw and saved hashes and original local paths are recorded. Strict Clippy, formatting, generated contracts, registered boundaries, selected Python lint/format and ShellCheck pass. Cached cargo-deny and cargo-audit pass across 227 dependency packages; the advisory database was not refreshed locally.

All **14 deliberate rejection probes** pass, including unregistered crates. The first attempt stopped at systemd verification because the sandbox denied its local credential sockets. Its transcript is retained; the successful rerun used local host sockets outside the sandbox and a disposable source copy. No system packages or production services were changed by these probes.

All ten GNU tar fixtures regenerate byte-identically in a temporary directory using tar 1.35, zstd 1.5.7 and gzip 1.13. Fresh archived-source frontend installation, eight tests and production build pass; their logs are retained. The original engineer's 214-test transcript and memory observations remain tied to that branch, rather than being relabeled as measurements of the combined runtime.

## Integrated inspector memory

The release example built from the frozen runtime is 812,456 bytes, SHA-256 `eb21aab14655df4c3708fd4bf7fdc0f0561dbc8343a6020280d2f67469e3fbe7`. `measure_inspector.py` records three fresh-process samples per archive, under the original synthetic measurement policy with a 2 MiB zstd window. The result records full manifests, archive/file checksums, policy/binary hashes and platform. Random payloads are generated in a temporary directory and checked against the resulting file digest.

| Archive | Peak RSS range (KiB) |
|---|---:|
| Small gzip | 2,620–2,680 |
| Small zstd | 2,840–2,892 |
| Legacy primary repeats, zstd | 2,764–2,824 |
| 64 MiB zeros, zstd | 4,992–5,136 |
| 64 MiB random, gzip | 2,680–2,756 |
| 64 MiB random, zstd | 4,800–4,944 |

The measured peak is about 2.6–5.1 MiB across these inputs. This is workstation process RSS for the standalone inspector; it makes no installed-service PSS, Pi, latency or Python-comparison claim. Production inventory limits remain undecided.

## Frozen native preparation

`source-manifest.json` binds runtime, fixture and frontend asset bytes. The prepared bundle is `.cache/arm64-qual/2026-10-06-integration/source.tar.gz`, **1,688,191 bytes**, SHA-256 `669675abab7b60c0a645ef70531dcd4890a144a681ee2b35c802af870069c594`. Intended identity: ARM64, authority schema 8, Debian qualification version `0.4.7+arm64.1`, native KVM. This is a source bundle, not a built or qualified package.

The earlier [60e6309 preparation](../../arm64/2026-10-06-current-60e6309-preparation/README.md), bundle and genuine schema 6 artifact provenance remain intact. An isolated ARM64 SSH target is still required. No SSH, test, measurement or qualification command ran on wybie or Holly's production Pi during this integration. The frozen Python project was only inspected locally.

`previous-schema7-artifact.json` also prepares the genuine 0.4.3 ARM64 artifact from source `f39396b`, recorded in CI 37386421570. Its package control, original package/four binary hashes, AArch64 ELF headers and original source schema are verified locally. This supplies a schema 7 → 8 upgrade candidate without relabeling current binaries. Neither that upgrade nor the schema 6 → 8 upgrade has run for the integrated bundle.

[CI run 37526173585](https://github.com/Brownster/limeos/actions/runs/37526173585) checks this exact runtime on native AMD64 and ARM64 and runs installed acceptance in disposable AMD64 Debian guests. CI results, when available, qualify their recorded source and workload; they do not substitute for Pi footprint measurements.

Both native jobs passed all 231 Rust tests, nine harness tests, dependency/security checks, contracts, boundaries, frontend and package gates. The ARM64 job compiled and exercised the new libzstd/inspector natively. `ci-native.json` binds the completed job metadata, original log hashes, three packages per architecture and all four binary hashes/ELF architectures. Raw logs are retained losslessly as `.log.gz`. Standard 0.4.4 and 0.4.4+ci.1 labels contain the same current binaries; they do not prove a historical upgrade. Current installed native ARM64/Pi qualification is still pending; the disposable AMD64 guest workflow was still running at this capture.

Each engineer assignment retains its 24-hour engineering estimate and review threshold of 36 hours. Agent wall time is not a human engineering estimate. P04 retains 320 hours with a cutover-scope review at 480. No defect-register row closes here. RW-040 protected physical/pool/protection/share dependencies and mount/fstab operations remain the next integration work.

## Completed installed acceptance

The complete workflow subsequently passed. `ci-workflow.json`, `ci-acceptance.json`, lossless `ci-vm.log.gz` and the five full guest results retain that outcome. Fresh Debian 12 AMD64 guests passed **180 acceptance groups**: foundation 14, observations 6, reference/container 61, approved storage 50 and container dependencies 49. Installed storage/dependency binaries match the native build hashes. These suites use current-source version labels and record no genuine historical upgrade; the prepared schema 6/7 upgrade candidates remain untested.

After authentication, core used 6,341 KiB PSS and the three base services together used 9,355 KiB PSS, with zero swap. After the container lifecycle/interruption workload, the dependency-suite snapshot was 12,026 KiB combined PSS with overview p95 0.824 ms, before the new dependency reads. These are AMD64 guest observations; Docker, fixture/proxy/build tools, optional storage services, password workers and the assistant are excluded. Current installed native ARM64/Pi and a comparable frozen-Python workload remain pending.
