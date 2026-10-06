# Engineer 1: current ARM64 packages and footprint

Qualify the integrated schema-8 build on an isolated native ARM64 test host while the integrator continues RW-040. The earlier ARM delivery proves older sources; current native CI passes 185 tests but does not measure installed services. This assignment supplies that missing evidence.

Start a branch `engineer/current-arm64-qualification` from `60e6309384c6a93caa63c0d578dc57d981897863`. Freeze that source for the first run. Estimate: 24 engineering hours with a test host available; record infrastructure waits separately. At 36 hours, deliver completed evidence and review the remaining scope with the integrator. The P04 phase estimate remains 320 hours, with its existing review threshold at 480 hours.

Read the [previous handoff](2026-10-05-engineer-arm64-qualification.md), [received report](../rewrite-evidence/arm64/2026-10-06-native-arm64-report.md), [qualification harness](../../tests/qualification/arm64/README.md), [current CI identity](../rewrite-evidence/p04/native-ci-approved-integration.json), [approved target boundary](../adr/0010-p04-approved-storage-targets.md), and [architecture budgets](2026-10-04-rust-rewrite-architecture.md).

## Deliverables

1. Update the existing ARM harness to take explicit source/package identities and qualify schema 8. Its build, install and upgrade scripts currently hardcode 0.4.2. Keep historical results intact and give new packages a distinct qualification version and artifact directory. A fixture change belongs to a separately identified fixture commit; record the runtime source independently.
2. Produce native Rust 1.88 locked build, workspace tests, strict Clippy and generated-contract results, plus standard/shadow ARM64 Debian packages and SHA-256 source, binary and package manifests. The frozen base contains 185 tests; report the actual count for each tested source. Preserve the signed test-repository installation path.
3. Prove installed standard/shadow identity, root-owned executable paths, capabilities and dormant optional units, both imported password formats, durable sessions and shadow write refusals. Run the current approved-storage and container suites in guarded disposable guests, including approval expiry/staleness, resource conflicts, interrupted jobs, receipt reconciliation and actual package teardown. A shadow removal must preserve the active standard target service; a standard replacement/removal must stop it.
4. Prove at least one genuine older ARM64 package upgrade to the new payload. Bind the old artifact hash and its original source; preserve sessions, pending jobs, receipts and resource claims applicable to that old schema. Only claim schema-7-to-8 coverage if a genuine schema-7 ARM64 artifact is actually available and tested. Rebuilding new code under an old version label proves no historical upgrade.
5. Repeat footprint measurements on the exact corrected, integrated payload: per-service and total PSS/RSS/swap; ten-minute idle CPU; readiness and short-lived task timings; bounded authenticated read latency; installed bytes; and bytes written over a stated interval. Report base services, optional storage readers/target service and password workers separately. Record CPU, page size, kernel, RAM, storage, enabled services, host load and workload. Mark daily write extrapolations as estimates. Compare with each applicable architecture budget and retain raw samples.

The previous 9.68 MiB observation belongs to the uncorrected frozen source, not this build. Native ARM CI, a native KVM guest, bare-metal Pi measurements and a comparable Python baseline are separate claims. Report the environment actually used.

## Ownership and isolation

Own `tests/qualification/arm64/` and a new run directory under `docs/rewrite-evidence/arm64/`. Reuse the current [approved-storage fixture](../../tests/privileged_vm/p04_approved_guest.py) and [container fixture](../../tests/privileged_vm/p04_approved_container_guest.py), including their helper modules. Keep any necessary shared-fixture adaptation small and in a separate commit; preserve its failure assertions. Return runtime defects as reproductions with an independent minimal fix for integration.

Use a spare ARM64 Debian test machine or disposable native ARM64 guests on an explicitly available test host. This handoff grants no new quiet window on wybie or Holly's production Pi. Keep the Python project frozen. All write, disk and interruption tests use empty synthetic disks with the existing identity guards; expose no host data directories or boot disks. Reuse the previous harness's memory limits and record host memory pressure.

Keep main, runtime APIs, database migrations, service policy and dependency pins unchanged. Return a reviewable branch with reproducible commands, raw successful and failed results, exact identities and an explicit list of untested gates. Hardware availability must not block the integrator's storage work.
