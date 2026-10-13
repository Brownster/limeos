# Declared storage consumer graph

`limeos_domain::storage_dependency_graph::configured_storage_dependency_graph`
composes the merged configured storage and Samba planners. It takes the storage
contract, pools, protection, Docker declarations, share configuration and caller
time, then returns a checked graph or fixed-field findings. It acquires no clock,
file, descriptor, process, lock or authority.

The independent `ConfiguredStorageGraphPreview` format has version 1 and supports
serialization only. Existing standalone formats/functions stay unchanged. It
contains original Docker engine/observation time, numeric media ownership, one
desired share configuration, its unmapped-share list and ordered device rows.
Each row contains the complete existing `ConfiguredDeviceDependency` plus
`ConfiguredShareConsumer { share_name, via }` records. Exact unique names refer
to the retained desired share configuration; comments/user lists are stored once.

Share requirements join by complete expected device metadata, including role,
UUID, filesystem, mountpoint and serial. Branch order, generated protection
requirements, full container identity/source/destination, desired share order
and all flags survive. Device/share-consumer ordering is deterministic. Disabled
or read-only shares, disabled pools/protection and stopped/read-only containers
remain possible dependents. Docker time is preserved without renewal.

One 512-match limit covers all expanded share and container routes. The existing
individual limits also apply. A 490-route container preview plus 23 share routes
is refused even though both standalone previews qualify individually. Exact
511 and 512 mixed totals pass; 513 returns no partial graph. Static configured
pool/protection records retain their existing separate bounds.

The [12 regression cases](rewrite-evidence/p04/rw040-configured-graph/README.md)
cover four independent existing-layout goldens, unchanged full storage rows,
disabled/stopped/read-only consumers, ancestor/direct/pool routes, ordered
branches, full metadata and numeric ownership, named volumes and original time,
single-copy share settings, case/component boundaries, enumeration order,
invalid/stale input, mixed limits and input immutability. All validation failures
remain sanitized; an invalid disabled configuration cannot produce a partial
view. Stopped container fixtures retain their required original start timestamp.

This graph is a configuration relationship input to the integrator's future
unmount/runtime-loss plan. It supplies no proof of actual Samba/global/account
state, share/provider completeness, mount/namespace/UUID/backing identity,
symlink/bind equivalence, source existence or retained freshness. An unmapped or
empty row does not prove absence of dependents. Caller-declared empty shares
remain caller input, never authoritative discovery. Frame-byte/service admission
and complete protected source/namespace collection must enforce their own limits
before exposure or effects; this delivery raises no service ceiling.

The integrator must bind trusted configuration revisions and protected evidence,
claim every affected device/mount/pool/protection/share/container resource,
revalidate at effect time and use approvals, storage locks and recovery. No live
operation, schema, HTTP route, CLI, UI, wire type or dependency is added here.
DSK-001, MNT-001/MNT-002, RT-001, SRA-001/SRA-002, UI-003, BKP-001, P04 and cutover
remain open. Tonight's integrated Pi candidate is frozen separately and excludes
this unmerged branch.
