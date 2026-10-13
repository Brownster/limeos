# RW-040 combined reader resource admission

Engine, process and physical-storage profiles now bound their own collection.
Configuration remains on the larger host parser and typed JSON can expand before
domain validation. Close that gap, then deliver reusable bin-library admission
for the future supervised worker. This slice does not launch or enable collection.

## Configuration prerequisite

Add the trusted `ConfigurationReadProfile::CombinedV1` with fixed 16 KiB files,
32 KiB total raw bytes, 33 retained descriptors and a 12 KiB metadata report.
Use the closed storage host-mount profile in acquisition and validation. Existing
numeric-limit callers remain Standard. Before typed decoding, visit all four
protected raw inputs with one non-materializing JSON admission budget: 2048
nodes including keys, depth 16, 128 sequence items, 32 map entries, 64 KiB decoded
key/string bytes and 4096 bytes per string. Reject excess collection tokens before
decoding them. Standard parsing and domain/cross-configuration rules remain.
Keep the original configuration lease, descriptor/name checks and poisoned state.

## Budget and lifecycle

Add an unregistered `limeos_executor::read_budget` library. Its closed profile
selects the Engine, process, storage and configuration CombinedV1 adapters and
the existing 32-destination/32-descriptor/32 KiB destination limits. No public
numeric override, wire selector, UID override or kernel-root selector is added.
Plans and reports describe admission; they cannot establish retained facts or
authorize storage effects.

| Retained owner | Maximum descriptors | Modeled payload envelope |
|---|---:|---:|
| Configuration | 33 | 12 MiB |
| Engine | 1 | 2 MiB |
| Processes | 37 | 2 MiB |
| Physical storage plus sources | 75 | 2 MiB |
| Destinations | 32 | 1 MiB |
| Configured graph work | 0 | 2 MiB |
| Final borrowed report buffer | 0 | 256 KiB |

All retained owners total 178 descriptors. Reserve 16 for the fresh worker
baseline and 32 for transient work, giving 226, below the service's 256 ceiling.
Require the independently launched worker to have its own soft descriptor limit
set to exactly 226 before admission; refuse a different limit. This library
only reads that limit. Tests may lower it in their own isolated child process;
the supervisor's later launch implementation must establish it before adapters.

The payload model totals 21.25 MiB, inside a fixed 24 MiB envelope. Admit a fresh
worker baseline of at most 24 MiB RSS and four threads. The service retains
MemoryMax=64M, MemorySwapMax=0 and TasksMax=16. Baseline plus modeled payload is
48 MiB, leaving a modeled 16 MiB margin. This is a conditional planning model,
not a total-heap/RSS/cgroup guarantee: allocator metadata/abort, library mapping,
runtime growth, child memory and unmeasured implementation scratch need installed
qualification. Never turn a sampled RSS value into a claim of peak enforcement.

The configuration envelope conservatively allows typed cells and Vec growth for
the admitted JSON shape, decoder scratch, raw/typed copies, bounded host tables
and planning work. Pin the known typed configuration node sizes below 1024 bytes
in regression tests. A changed shape/toolchain/serde implementation requires
re-audit. Other adapter payload ledgers and cardinality caps remain the primary
enforcement; the model does not replace them.

Read fixed own-process status/fd paths with bounded input and token iterators.
Require a valid descriptor limit and bounded baseline before any phase. Reserve
the complete retained envelope for each phase before it begins, keep previous
reservations, and permit only Configuration → Engine → Processes → Physical →
Destinations → Graph. A lexical phase guard prevents overlapping phases and
poisons the session if dropped without committing. Fresh worker samples after
each phase check descriptor use against baseline plus retained reservations and
transient allowance, RSS against the modeled ceiling, threads and unchanged
descriptor limit. Failure poisons admission; it never becomes an empty success.

After all phases, borrowed serialization fallibly reserves one fixed 256 KiB
buffer and refuses excess before append. It returns bytes only after complete
serialization and a final worker check. These bytes are capacity-checked
observations, never fresh collection proof or effect authority. Actual owner
collection and original-age revalidation must be supplied by the later collector.

A separate nonblocking single-flight guard allows one pending worker per manager.
Its owner must retain the guard until the later supervisor reaps its child and
descendants; current RAII alone does not prove process cleanup. No queue or
independent deadline is claimed here.

## Alternatives, validation and remaining work

Keep admission in the bin library, where both adapters are already dependencies.
Avoid cross-adapter dependencies and a public command/RPC. Arithmetic alone
would leave the configuration parser gap, report allocation and phase overlap
unchecked; concatenating maximum owners would exceed the original limits.

Test configuration shape boundaries and all frozen layouts locally, admission
phase/failure/resource boundaries, nonblocking contention, bounded report output,
and own-process admission under a lowered limit in an isolated owned test child.
Compile-fail checks preserve phase/permit ownership and profile import boundaries.
Use private synthetic resource samples only in unit tests, never production
injection. Run workspace Rust, strict all-target Clippy, fmt, contracts/repository
checks and the local ARM64 harness; retain exact hashes and lossless logs.

No Pi, Docker, block probe, reserved backup recovery work or external qualification
paths are used. Build/general scratch stays on disk; precompiled existing storage
kernel fixtures may use small owned and cleaned tmpfs scratch. Next is genuine
owner composition in an independently supervised worker, original-age checking,
termination/cancellation/parent-death/descendant reaping, then installed resource
qualification. Claims, locks, effects, recovery and cutover remain open.
