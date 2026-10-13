# RW-040 process admission for the combined reader

The standard process collector retains up to 261 descriptors, before its
temporary readers or any other owner. Its maximum-row fixture exceeded the
storage reader's 64 MiB ceiling. The combined worker therefore needs enforced
adapter admission before composition. This slice supplies the process portion;
it does not enable a worker or qualify whole-worker memory.

## Choice and boundary

Add a closed `ProcessReadProfile::CombinedV1` to the trusted Rust library, beside
`Standard`. Keep the existing public collection function on Standard. A new
profile-selecting function uses the same Linux authentication, fixed proc paths,
pidfds, retained descriptors, observations and original dual-clock lease.
Profiles have no numeric overrides, serialization, RPC or command selection.
Retained owners store the chosen profile for every subsequent validation.

Reducing all callers' limits would change existing advisory behavior. A budget
planner without enforcement would leave parser amplification and descriptor
opens unchanged. The additive profile addresses the actual collection seam.

## Closed process bounds

CombinedV1 admits at most 16 complete declarations, eight running consumers and
32 declared mounts, including stopped declarations and mountless processes.
Validate the whole set before kernel opens or copying declarations. Never clip
membership or mount rows. These counts agree with the existing Engine profile.

Each complete host or process mount table is at most 64 KiB and 512 rows. Each
running-process pass admits at most 256 KiB of raw tables, charging repeated
namespaces separately. Initial, held and freshly named passes have separate
counters. Apply the same table/row limits to current and PID 1 host tables.
Check bytes and rows before reserving parsed vectors. Optional tokens remain
bounded to 32 without collecting a token vector.

Retained parsed mount vectors and their three string capacities share a 512 KiB
payload allowance across all running processes. Check requested vector/string
space before fallible allocation, then charge actual capacity, refusing allocator
rounding that exceeds admission. Decoded filesystem roots and paths are at most
512 bytes, filesystem names at most 64 bytes. Validate/count escaped paths before
allocating their decoded output. Whole-table host parsing uses a separate bounded
scratch allowance; it does not consume the retained-process allowance.

Declaration/binding serialization remains capped at 64 KiB and process reports
at 48 KiB. Five context handles plus four per process give 37 retained descriptors;
Linux validation needs at most four additional descriptors, for 41 in this
adapter. Other owners, transport, child probes, report buffers and runtime
descriptors still need a combined budget. No claim that 41 is the worker peak or
that counted mount payload proves RSS fits MemoryMax is permitted.

## Failure and evidence

Oversized input returns Unavailable, invalid membership keeps its existing
failure, and every failure drops partial handles. Validation under the original
profile cannot widen limits or renew the lease; a failed validation poisons the
owner. Existing capability, namespace, syscall, service and kernel guards stay.

Local private fixtures cover exact/excess declaration, running, stopped-mount,
table, row, decoded field, aggregate and payload admission; rejection before
opens; retained profile on validation; cleanup, expiry and standard compatibility.
Existing owned-child tests exercise real pidfd/proc/root/namespace handles with
an explicit private host-authentication substitution. They cannot qualify public
root collection, foreign namespaces or installed permissions. Record process
payload/descriptor measurements separately from whole-worker footprint.

Run focused and workspace Rust tests, strict all-target Clippy, fmt, contract and
repository checks, plus the existing local ARM64 harness. Build and general
scratch stay on disk; precompiled kernel fixtures may use a small owned and
cleaned tmpfs directory. Preserve lossless transcripts and exact source hashes.

Next: storage/topology/source/parser admission, a concrete combined resource
budget, then independent worker termination, single-flight admission, parent
death, cancellation and descendant reaping. Engine/configuration integration,
claims, storage locks, effects, installed ARM64 qualification and cutover remain
open. No Pi, Docker, raw storage probe or reserved backup recovery work is used.
