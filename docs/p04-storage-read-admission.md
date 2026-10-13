# Combined storage read admission

`StorageReadProfile::CombinedV1` is an additive trusted Rust selector for
`inspect_storage_inventory_with_profile`,
`inspect_container_sources_with_profile` and
`inspect_container_dependencies_with_profile`. The dependency entry point
preflights membership before probing and passes the same profile to both retained
parents. Each parent retains its profile for subsequent validation. Existing
entry points select Standard; there is no numeric, serde/wire, CLI or RPC selector.

| CombinedV1 bound | Scope |
|---|---|
| 16 complete consumers / 8 running / 32 declared mounts | Includes stopped mounts and running mountless consumers |
| 64 KiB / 512 complete host mount rows | Current and PID 1 tables, acquisition and validation |
| 512-byte decoded root/path / 64-byte filesystem name | Escape validation/counting before output allocation |
| 32 optional mount tokens | Fixed-field iterator; no token vector |
| 128 KiB vector/string capacities | Each parsed host table; actual capacities charged |
| 48 topology nodes / 8 parents per node / 128 total parent entries | Includes virtual nodes; refusal before excess parent metadata reads |
| 40 nonvirtual probes | Complete preflight before first device open; includes nodes without signatures |
| 64 KiB fstab / 128 six-field entries | Protected retained regular file; private options remain outside the report |
| 64 KiB swap input / 8 complete entries | Decoded names bounded to 512 bytes |
| 128 aggregate source mount references | Duplicate source declarations consume the same allowance |
| 256 pending components / 16 KiB string capacity | Symlink expansion admitted before copying |
| 64 KiB declaration input / 48 KiB reports / 256 KiB digest scratch | Bounded writers preserve existing digest bytes |

Mount bytes and rows are checked before vector reservation. Parsed vectors and
their three strings charge actual capacity against the per-table allowance.
Requested space is checked before fallible allocation; allocator rounding beyond
admission refuses. Fixed token arrays/iterators replace whitespace amplification.
Oversized or malformed input refuses complete collection; there is no truncation.
Raw complete mount-table digests still bind parent IDs, propagation, sources and
options beyond the selected parsed fields.

Topology validation retains missing/self-parent, cycle and depth refusal. The
combined profile memoizes DAG heights, so shared ancestors are computed once;
the longest supported path remains 16 edges. Parent admission checks the per-node
and aggregate bounds before reading the next parent identity. Probe admission
counts every nonvirtual node, before opening any device. The existing protected
`/dev` walk, block type/device checks, retained-descriptor blkid probes, duplicate
UUID behavior and complete boot/swap graph exclusions remain in place. The
synthetic 15 parent/partition pairs plus three boot nodes fit the count; an actual
frozen host layout and successful public root probes remain unqualified.

Sources retain complete declarations and one O_PATH descriptor per mount,
including stopped consumers. Candidate references are counted against the
remaining allowance before vector allocation. Pending ordinary-symlink targets
are admitted before queue expansion; popped string capacity is released from
that queue's count. Existing 40-link, 512-step and 512-byte path guards, cached
confined lookups and remote/FUSE refusal before inode metadata stay in place.
Revalidation compares held and freshly named objects under the original profile
and expiry. This slice preserves the existing owners' failure behavior; it adds
no independently enforced synchronous deadline or universal failure poisoning.

At most 40 block handles plus fstab give 41 retained storage descriptors.
At most 32 source handles plus two context handles give 34 retained source
descriptors. These are adapter retention bounds, excluding temporary walks,
fresh fstab handles, probe subprocess/transport pipes and other owners. The real
local source fixture measured 34 above its baseline and confirmed cleanup and
file-data read refusal. It uses a private nonroot collection seam; public root
authorization, block devices and installed permissions are not qualified by it.

The parsed-table allowance excludes raw buffers, other report/input copies,
BTreeMap/BTreeSet nodes, allocator metadata, path-walk scratch, runtime and stacks.
Bounded JSON writers count output before making an oversized JSON body. Their
successful digests match Standard bytes. Counts and payload allowances here do
not prove whole-worker heap, RSS, descriptor headroom or elapsed performance.
The existing service's 256 descriptors, 64 MiB, zero swap and 16 tasks remain
independent final guards.

Local fixtures cover exact/excess membership, probes, table bytes/rows/fields,
parent/aggregate edges and DAG height, swap/fstab rows, reference and queue bounds,
retained profile/expiry, real source cleanup and standard compatibility. The
profile import compile-fail check enforces the wire boundary. Existing storage,
process, destination and frozen layout regressions remain in the full workspace.
No new caller-selected kernel root, UID override, dependency, service/capability
change, storage effect or worker entry point is introduced.

Next is the concrete budget across all retained owners, then independent worker
termination, single-flight admission, cancellation, parent death and descendant
reaping. Engine/configuration integration, nested and unselected physical
consumers, complete claims, actual pool/share state, locks, effects, recovery,
native/installed ARM64 qualification and cutover remain open. No Pi or Docker
operation or raw workstation storage probe is included.
