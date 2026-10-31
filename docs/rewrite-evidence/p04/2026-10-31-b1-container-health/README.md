# Retained container health: local qualification

Source `7f1256d889beb9db6a66876bb6b1c913d589428e` implements the
[design](../../../plans/2026-10-31-b1-container-health-evidence-design.md) and
[handoff](../../../p04-b1-container-health-evidence-handoff.md). This packet
qualifies the genuine authenticated HTTP collector against owned Unix fixtures.
Successful fixtures use private unit-only endpoint credentials and clocks.
They do not establish successful installed UID-0 Docker, host namespaces,
protected worker/RPC registration, API/UI or cutover acceptance.

## Scope and behavior

The fixed public entry requires effective UID 0 and the canonical root-owned
`/run/docker.sock`. Each HTTP connection authenticates its kernel peer before
request bytes. Health is selected in the original complete double inspection,
with complete membership/Engine identity bracketing both passes. The retained
owner shares original socket identity, storage/PID facts, five-second dual-clock
lifetime, collection/request deadlines and spent byte/payload ledger.

Restart policy and effective running/healthcheck state produce frozen-rule
signals bound to full container IDs, with selected display names in summaries.
Configured missing/unknown health is unavailable. Malformed and contradictory
facts refuse. Commands, health logs, environment, unrelated labels and
credentials do not enter reports. Original and health reports share the
48-KiB limit; 16-consumer/eight-running/32-mount and other service ceilings stay
unchanged. These counts are not an installed capacity or total-heap guarantee.
Container-signal completeness makes no claim about the remaining health sources.

The shared permanent validation gate keeps failure, abandoned work and
concurrent re-entry poisoned, including through the borrowed storage owner.
Old storage-only/Compose selections and generated public contracts remain
unchanged. Serialized snapshots cannot restore the retained authority.

## Exact local checks

The final affected run passes **986 runtime tests and 33 compile-fail doctests**:
562 domain, 49 executor-library, two executor-main, five default combined-probe
and 368 container-library runtime passes; seven domain, seven executor and 19
container-library compile-fail passes. Eleven new native health tests cover
real double-inspection/revalidation transport, stopped/mountless membership,
policy/health/state rules, configured missing results, secret omission, second
pass changes, permanent invalidation, cancellation/concurrency, original clocks,
unchanged bounds/report amplification and production peer refusal before HTTP.
Two new compile-fail cases prevent importing the health owner/snapshot.

The outer harness marks two existing child-entry helpers ignored:
`journal_capacity_probe` and `mount_runtime_child`. Their respective capacity
and runtime-bootstrap parent regressions invoke them in isolated child processes
and pass. No skipped installed/hardware suite is promoted. Opt-in combined
launch/supervision feature suites are not enabled by this run.

Strict Clippy covers container evidence and dependent executor packages with
all default-feature targets. Actual contract regeneration matches the existing
schemas/TypeScript. Formatting, repository hygiene and diff checks pass. No
frontend source or public contract changed, so no new frontend/browser journey
claim is made.

Three final guards bind the same **724 committed input hashes**. `source-match.json`
checks those bytes against the exact feature commit and working tree. All four
attempts and original launch/check logs are retained. The first Cargo run
passed its tests but changed inputs during development; it does not qualify the
final source. The final frozen run includes the joint-report and additional
malformed-selector regression. No failed or changed-input draft is promoted.

All scratch is under the repository, using one Cargo job/test thread, a 4-GiB
start threshold and the unchanged sampled 3-GiB MemAvailable floor. Guard records
retain actual minima and original process IDs. Native final inspection finds
all original guard/child IDs absent. No other process or service was stopped.
`SHA256SUMS`, the source patch, reproducible changed-source archive and closed
launcher scripts retain exact inputs; inspect/adapt their repository paths
before running them elsewhere.

`reference-inputs.json.gz` pins read-only inspected frozen Python source at
`80593b29443ea9b81eb60ed4e5c3c3ac7a236204`, including matching file hashes.
The older empty restart-name interpretation is supported by
[Moby v28.0.0's definition](https://github.com/moby/moby/blob/v28.0.0/api/types/container/hostconfig.go).
This reference establishes grammar, not the installed Engine version. No
Python service or source was changed or executed.

## Remaining acceptance

The protected worker must adopt this owner, preserve remaining original lifetime
across its authenticated reply and resolve current concrete scopes before
feeding the transition-only Store seam. Legacy name/silence and replacement/
retirement binding remain open. Fresh SMART/mount/host/protection aggregation,
fair scheduling, safe compaction, ordinary external notifications, protected
actual-state migration, installed API/UI/restart/real idle-write evidence,
current packages/upgrades/return paths and ARM64/Pi resource/footprint acceptance
remain open. B1, A1 reliability, wider A/B/P04 and P05/P06 cutover are unfinished.
No phase or defect is closed and no inventory performance target changes.
Holly's Pi, actual Docker, deployed services, external recipients and the frozen
app were untouched. Nothing was pushed.
