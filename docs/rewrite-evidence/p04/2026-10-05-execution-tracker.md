# P04 execution tracker

Status: local implementation started after P03's 62-group representative-layout suite passed. The operator authorized continuing P03 then P04 on 2026-10-05. All implementation happens in this Rust repository; the Python checkout is frozen. Mutation tests use disposable VMs. Native ARM64 and reference-host measurements remain separate outstanding qualification gates.

Estimate: forty implementer days (320 hours) for RW-040 through RW-045, including their API/UI screens and regression evidence. Review the cutover scope at 480 hours. Allocation: 72 hours for storage identities, mount planning, boot exclusion and mount-loss safety; 64 for pools/protection/remote mounts/shares/prerequisites; 64 for catalog/Compose/media paths and tools; 48 for encrypted backup/restore and recovery; 32 for package/container update transitions; 40 for schedules/alerts/notifications and restart recovery. Record actual agent work and approval waits separately; neither is a measured human implementer-day figure.

The first slice ports the frozen storage contract into typed, generated domain contracts before registering a host effect. [ADR 0006](../../adr/0006-p04-storage-contract.md) defines its boundary. Live disk identity/topology, mount namespace, descriptor traversal and installed-unit deadlines must be qualified before any storage mutation is enabled.

| Work package | Current status |
| --- | --- |
| RW-040 — Storage identities, contracts and mounts | Pure storage contract and bounded device-readiness plans implemented; live identity/topology, boot exclusion, host adapter and UI remain pending. |
| RW-041 — Pools, protection, remote mounts and shares | Pending. |
| RW-042 — Catalog, Compose and media deployment | P03 preview foundation exists; deployment execution, live imports and UI remain pending. |
| RW-043 — Backup, restore and recovery | Pending. |
| RW-044 — Package/container updates and host prerequisites | Pending. |
| RW-045 — Schedules, alerts and notifications | Pending. |

Representative capture found a literal Navidrome `/data"` bind target beside an anonymous `/data` volume, case-sensitive `/mnt/storage/TV` and `/mnt/storage/Movies` paths, and VPN capabilities/shared network namespaces. Import/deployment must report and preserve unsupported or malformed settings rather than silently flatten them. No correction was made on wybie.

No P04 defect-register row is closed. Destructive/storage tests require realistic virtual disks, boot protection, replacement/UUID races, lost mounts, dependent shutdown, concurrent edits, mount namespace checks and archive/restore failure scenarios. Schedules and notifications must work with all model providers disabled. The phase cannot complete until each existing feature has a mapped acceptance scenario and every assigned defect regression passes.

## First storage slice

Commit `9d38d5d` ports the frozen schema-version `"1"` contract into the pure domain crate. [Golden fixtures](../../../tests/fixtures/storage-contracts.json) preserve single-disk, separate-downloads and fifteen-drive protected-pool cases, including the original device order, numeric UID/GID, filesystem set, UUID/serial and fixed container paths. Parsing never normalizes case or changes data locations. Unknown fields, duplicate assignments, invalid ownership/role counts, escaping paths and unsupported versions fail validation. Generated JSON/TypeScript describe the same wire shape, including optional serial numbers; cross-field/profile checks remain authoritative in the domain validator.

Readiness plans contain only assigned device mountpoints and identities, sorted independently from the stored contract. The requested timeout must be 1–120 seconds. Media/download/backup subdirectories are not added as waits. A protected pool's virtual `/mnt/storage` root or descendants cannot masquerade as physical backing assignments. This is planning only: the host adapter must observe actual mount identity and enforce the deadline. MNT-001 remains open until an unsatisfiable installed wait is shown to time out. No storage intent, RPC effect, HTTP mutation or unit was added.

Eight [storage regressions](../../../crates/domain/src/storage/tests.rs) pass. The full [workspace transcript](rust-contract-tests.txt) records 106 passing tests; strict Clippy, formatting, generated-contract drift, repository boundaries and frontend TypeScript checks pass. Cached advisory/license/source checks pass across 214 dependencies. [Validation metadata](storage-contract-validation.json) records the source commit and exact check scope. These are source checks; no new P04 package or footprint claim is made.

The first workspace run exposed an existing legacy PBKDF2 deadline failure. [Its transcript](rust-contract-first-run.txt) is retained. Commit `a02b271` uses the already-pinned Ring PBKDF2 verifier, with unchanged salt/algorithm/iteration bounds and the same eight-second/two-worker limits. The current core login/upgrade regression passes for both Werkzeug formats, wrong passwords and successful-only rehash. [The password boundary review](password-boundary-review.md) records the native dependency change, and [timing observations](password-timing.json) record the limited workstation workload. No new third-party crate was introduced. Fresh package and native ARM64 qualification must include this correction; frozen 0.3.2 artifacts remain unchanged.

This slice started at approximately 16:56 UTC and completed after the long test approval and local compilation/diagnosis window. Agent/build activity and approval waits are recorded separately in validation metadata; neither is a measured human implementer-day figure. The 320-hour estimate and 480-hour scope-review threshold remain unchanged. Wybie and the Python checkout received no mutation, installation, benchmark or service change.

Next RW-040 work is fresh disk/parent topology and mount identity, explicit root/boot exclusion, protected path traversal, and real bounded waits on virtual disks. Unmount and mount-loss behavior must include dependent containers, shares, pools and protection paths before enabling effects. Other P04 work packages and their screens remain pending.
