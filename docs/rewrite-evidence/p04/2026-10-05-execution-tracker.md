# P04 execution tracker

Status: local implementation started after P03's 62-group representative-layout suite passed. The operator authorized continuing P03 then P04 on 2026-10-05. All implementation happens in this Rust repository; the Python checkout is frozen. Mutation tests use disposable VMs. Native ARM64 and reference-host measurements remain separate outstanding qualification gates.

Estimate: forty implementer days (320 hours) for RW-040 through RW-045, including their API/UI screens and regression evidence. Review the cutover scope at 480 hours. Allocation: 72 hours for storage identities, mount planning, boot exclusion and mount-loss safety; 64 for pools/protection/remote mounts/shares/prerequisites; 64 for catalog/Compose/media paths and tools; 48 for encrypted backup/restore and recovery; 32 for package/container update transitions; 40 for schedules/alerts/notifications and restart recovery. Record actual agent work and approval waits separately; neither is a measured human implementer-day figure.

The first slice ports the frozen storage contract into typed, generated domain contracts before registering a host effect. [ADR 0006](../../adr/0006-p04-storage-contract.md) defines its boundary. Live disk identity/topology, mount namespace, descriptor traversal and installed-unit deadlines must be qualified before any storage mutation is enabled.

| Work package | Current status |
| --- | --- |
| RW-040 — Storage identities, contracts and mounts | Contract and bounded readiness-plan design accepted; implementation in progress. |
| RW-041 — Pools, protection, remote mounts and shares | Pending. |
| RW-042 — Catalog, Compose and media deployment | P03 preview foundation exists; deployment execution, live imports and UI remain pending. |
| RW-043 — Backup, restore and recovery | Pending. |
| RW-044 — Package/container updates and host prerequisites | Pending. |
| RW-045 — Schedules, alerts and notifications | Pending. |

Representative capture found a literal Navidrome `/data"` bind target beside an anonymous `/data` volume, case-sensitive `/mnt/storage/TV` and `/mnt/storage/Movies` paths, and VPN capabilities/shared network namespaces. Import/deployment must report and preserve unsupported or malformed settings rather than silently flatten them. No correction was made on wybie.

No P04 defect-register row is closed. Destructive/storage tests require realistic virtual disks, boot protection, replacement/UUID races, lost mounts, dependent shutdown, concurrent edits, mount namespace checks and archive/restore failure scenarios. Schedules and notifications must work with all model providers disabled. The phase cannot complete until each existing feature has a mapped acceptance scenario and every assigned defect regression passes.
