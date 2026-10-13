# RW-040 storage and source read admission

The Engine and process profiles now admit 16 complete consumers, eight running
processes and 32 declared mounts. Physical dependency collection still uses
larger storage/source owners and host parsers. Bound that complete collection
chain before a combined worker is built; leave its runtime entry point disabled.

## Additive profile

Add the closed trusted-library `StorageReadProfile::{Standard, CombinedV1}`.
Existing inventory/source/dependency entry points keep Standard. New profiled
entry points propagate one profile to both physical parents, and each owner
retains it for validation under its original lifetime. No numeric overrides,
serde/wire selector, caller-selected sysfs/proc/device root or authentication
substitution is public. Namespace, protected-path, cached lookup, boot/swap and
filesystem guards remain in place. No service or capability ceiling changes.

Changing all advisory callers' bounds would break existing behavior. A count-only
budget would leave mount token amplification and topology/source expansion
untouched. Choose enforced additive bounds at the real read/parse/open seams.

## Closed bounds

CombinedV1 host mount tables are 64 KiB and 512 complete rows, with decoded roots
and paths at most 512 bytes, filesystem names 64 bytes, and 32 optional tokens.
Before parsing, check bytes/rows and reserve vectors fallibly. Use token
iterators and validate/count escapes before allocating decoded output. Charge
mount vector/string capacities against a 128 KiB allowance per retained table;
allocator metadata, raw buffers and other owners are separate.

Topology admits 48 nodes, eight parent entries per node and 128 total parent
entries, rejecting before excess parent insertion/read. Memoize complete DAG
height validation to prevent exponential path enumeration; refuse cycles,
missing/self parents or depth above 16. Count all nodes, including virtual nodes.
Preflight all non-virtual probe candidates before opening the first device:
at most 40 retained probes, plus the retained fstab descriptor. Forty allows
15 parent/partition disk pairs plus boot nodes within this explicit subset;
it is no qualification of an actual frozen host layout.

Fstab remains a 64 KiB protected file with at most 128 six-field entries. Swap
input remains 64 KiB but admits eight complete entries for this profile. Parse
these fixed fields without collecting unbounded token vectors. Their string
fields are bounded before copying. Reports and input digests serialize through
bounded writers, rather than allocating an oversized JSON buffer first.

Source membership matches 16/8/32, including stopped consumers and mountless
running consumers. Admit before context/source opens and declaration copies.
Retain at most 32 sources plus two context descriptors. Admit at most 128 total
source mount references, including repeated source paths, before allocating the
next candidate vector. Bound symlink traversal's pending queue to 256 components
and 16 KiB of string capacity; check target expansion before copying. Existing
40-link/512-step/512-byte path guards still apply. Standard traversal is retained.

The storage and source owners keep their profile, original leases and current
revalidation rules. This slice adds no claim that older owners poison every
failure or independently terminate synchronous work. Dependency collection
preflights membership before probing and passes CombinedV1 to both parents.

## Acceptance and remaining work

Use local pure/private fixtures for exact/excess table, field, parent, graph,
probe, swap, source/reference and traversal bounds; preserve Standard regressions.
Real scratch source descriptors test read-only collection and cleanup through a
private nonroot test seam. Do not probe workstation block devices or contact Pi,
Docker, or the reserved backup recovery worktree. Public root/device/foreign
namespace and installed permissions remain unqualified.

Run focused/workspace Rust tests, strict all-target Clippy, fmt, contract and
repository checks plus the existing local ARM64 harness. Keep compilation and
general scratch on disk. Compile storage kernel fixtures before their owned,
auto-cleaned tmpfs runtime check. Preserve exact source hashes and lossless logs.

Next is the concrete budget across Engine/process/configuration/storage/source/
destination owners, followed by independent termination, single-flight admission,
parent death, cancellation and descendant reaping. No payload or retained-FD
count here proves whole-worker RSS or descriptor headroom. Engine/configuration
integration, complete affected consumers/claims, actual pool/share state, locks,
effects, recovery, installed ARM64 qualification and cutover stay open.
