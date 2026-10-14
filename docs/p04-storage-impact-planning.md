# Declared storage impact planning

`limeos_domain::storage_impact::configured_storage_impact` calculates selected
device dependencies and related resource candidates for review. It is a pure,
unregistered library: no file, clock, process, lock, approval or effect is acquired.
Its independent serialize-only format supplies no safe-unmount authority.

Pass the original storage contract, pools, SnapRAID configuration, Docker
declarations, desired shares, complete expected devices and caller time. The
planner rebuilds the [checked configured graph](p04-storage-dependency-graph.md)
from all inputs. Invalid unused or disabled configuration still refuses. Docker
declaration age is checked against its original observation time without renewal.
One to 32 distinct targets must match the current contract by full ID, role, UUID,
filesystem, mountpoint and optional serial. Unknown, changed, missing, duplicate
or excessive selections return fixed-field findings and no partial preview.

Selected graph rows identify possible container, share and pool dependents.
Stopped/read-only containers and disabled/read-only settings remain resource
candidates. `running_containers` contains full deduplicated original snapshots,
sorted by complete container resource. It is a review list; container IDs do not
establish Compose stack ownership. Direct paths, pool roots, ancestor paths,
literal case/component boundaries and named volumes retain the existing rules.

`selected_devices` and `related_devices` have different purposes. An affected
pool adds its ID, mount root and every configured member device/mount to the
resource candidates. Any affected SnapRAID drive adds one global protection
subject and every configured protection drive's device/mount. Related members
do not become unmount targets or add their direct consumers or other pools to the
affected-container list. For example, selecting `d1` can relate `d2` through their
pool while leaving a container using only `d2/Other` outside that list. Selecting
both devices includes that direct consumer. Shared physical backing and aliases
require later retained evidence.

`ConfiguredStorageImpactResource` has separate typed variants for device IDs,
mount names, pool IDs, global SnapRAID, share names and full container resources.
Names in different variants cannot collide. These subjects are independent of
Store claim keys and are not acquired claims. The complete original graph keeps
expected metadata, meaningful branch order, source/destination routes, flags,
desired share settings, unmapped shares, Engine identity, original time and
numeric media ownership. Device/candidate/running-list ordering is deterministic;
meaningful original configuration order is preserved.

The planner deduplicates before candidate count growth, accepts 128 distinct
subjects and refuses a 129th. The existing graph still applies its global
512-route limit before selection; targeting one device cannot bypass it. A
counting writer refuses a complete compact JSON preview above 256 KiB without
allocating a second report buffer. The already bounded typed graph is constructed
before that byte check; this supplies no peak allocation or service-headroom
proof. The existing workspace `serde_json` pin moves from domain test-only use to
runtime byte counting, with no dependency version change.

Every result has eight `evidence_needed` entries: retained identity/freshness,
complete physical consumers, actual pool/protection/share state, stack ownership,
complete claims/approval and effect-time validation. Empty declared dependencies
and unmapped shares cannot become evidence of safe absence. A future coordinator
must bind the protected configuration, physical aliases/backing, current runtime
state and actual storage/container/stack resources before claims or operations.
Approvals, storage locks and recovery remain in their existing workstream.
The [retained Compose membership reader](p04-compose-membership-evidence.md) now
supplies an authenticated declaration prerequisite. Actual protected stack files,
management identity and full claim binding remain required; this planner's
`StackOwnership` evidence boundary stays unresolved.

[Local acceptance](rewrite-evidence/p04/2026-10-14-storage-impact/README.md) covers
four independent frozen-layout goldens, full graph preservation, complete target
matching, related-member separation, stopped/disabled/read-only cases, literal
paths and volumes, ordering/deduplication, original age, invalid unused input,
512-route and 128/129-resource bounds, oversized valid reports, input immutability
and serialize-only output. The first failed fixture reused a pool name and had one
branch; corrected coverage uses two disjoint branches under the frozen rules.
See the [design](plans/2026-10-14-storage-impact-preview-design.md).

The combined collector/report, Store claims, schemas, routes, commands, service
guards and package executable lists are unchanged. Domain input bounds do not
qualify the smaller CombinedV1 worker profile or installed counts. Independent
review, root/foreign-namespace and installed qualification, actual pool/share
state, complete consumers/claims, effects/recovery, performance, P04/defect gates
and Pi cutover remain open. This slice is outside the frozen signed ARM64 candidate.
