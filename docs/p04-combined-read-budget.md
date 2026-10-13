# Combined reader resource admission

The unregistered bin library `limeos_executor::read_budget` supplies a resource
plan, nonblocking `SingleFlight`, fresh-worker `BudgetSession` and lexical phase
guards. The executor command, RPC and service launch stay unchanged. Admission
counts establish no successful owner collection, currentness or storage authority.

The closed configuration prerequisite is
`configuration::inspect_storage_configuration_with_profile(paths,
ConfigurationReadProfile::CombinedV1, cancelled)`. It uses fixed 16 KiB files,
32 KiB total raw bytes, 33 retained descriptors and a 12 KiB metadata report.
Acquisition and validation use the closed storage host parser. One shared
non-materializing JSON visitor checks all four protected inputs before typed
decoding: 2048 nodes including keys, depth 16, 128 sequence items, 32 map entries,
64 KiB decoded keys/strings and 4096 bytes per string. Excess collection tokens
are refused before their contents are decoded. Existing typed/domain rules and
original lease, name/descriptor checks and poisoned state remain. Standard
numeric-limit collection keeps its behavior. All four frozen layouts pass the
new profile through the private local collection seam.

| Phase | Retained FD reservation | Modeled payload |
|---|---:|---:|
| Configuration | 33 | 12 MiB |
| Engine | 1 | 2 MiB |
| Processes | 37 | 2 MiB |
| Physical inventory plus sources | 75 | 2 MiB |
| Destinations | 32 | 1 MiB |
| Configured graph | 0 | 2 MiB |
| Single final report | 0 | 256 KiB |

Maximum retained reservations sum to 178 descriptors. A 16-FD baseline and
32-FD transient allowance yield 226. Worker admission requires the launcher to
have installed exactly that soft descriptor limit; the library reads it and
refuses any other value. The installed service retains its 256 ceiling. A fresh
worker baseline must also have at most 24 MiB RSS and four threads.

The payload model totals 21.25 MiB, within a 24 MiB envelope. Baseline plus that
envelope is 48 MiB, leaving a modeled 16 MiB margin beneath MemoryMax=64M.
MemorySwapMax=0 and TasksMax=16 remain. RSS checks use bounded current samples,
with a 48 MiB refusal threshold after phases. These are not peak enforcement
or installed cgroup qualification. Allocator metadata/abort, runtime and library
growth, child memory and unmeasured scratch stay outside this conditional model.
The configuration envelope allows admitted typed-cell/Vec growth, decoder
scratch, raw/typed copies, host tables and planning work. The known typed cells
are pinned below 1024 bytes in a regression audit; changed shapes, serde behavior
or toolchains require re-audit. Existing adapter ledgers remain essential.

`SingleFlight::try_acquire()` refuses Busy immediately. Its guard must remain
owned by the future supervisor until the child and descendants are reaped.
RAII release alone proves no process termination. In a freshly constrained child,
`BudgetSession::admit_worker()` reads fixed own-process status/fd paths and the
descriptor limit, refusing unavailable or excessive samples. The fd sample
includes its observer directory, so it is conservative by one descriptor.
There are no production synthetic-resource, UID or kernel-root inputs.

Select a phase's closed adapter profile before calling `enter(phase)`. Entry
reserves that owner's full envelope while retaining previous reservations.
The required order is Configuration, Engine, Processes, Physical, Destinations,
Graph. Perform the actual adapter operation while holding the phase guard, then
`commit()` checks fresh resource counters. A dropped guard, skipped/repeated
phase or resource/sample failure poisons admission. Borrowing prevents overlapping
phases and dropping the session while a guard is held. Counter checks cannot
prove that an adapter was actually called; the future collector must own genuine
evidence and preserve every original age through full revalidation.

After all phases, `report(borrowed_observations)` consumes the session. It fallibly
reserves exactly one 256 KiB buffer, rejects excess serialization before append,
and checks fresh counters before returning `ReadReportBytes`. Serialization or
sampling failure returns no report. Returned bytes do not recreate an admission
session or a retained owner and confer no effect authority.

Local tests cover shape/descriptor boundaries and frozen configuration layouts,
retained profile/lease behavior, sequence and poison rules, exact/excess resource
samples, bounded escaped reports and nonblocking contention. An isolated owned
test child lowers only its own soft limit to 226, measures actual own-process
counters and exercises budget transitions without collecting owners. It proves
neither root authorization, child supervision, full collection nor footprint.
Its parent's descriptor limit is verified unchanged. Compile-fail tests protect
profile import, phase borrowing and single-use report serialization.

Next is genuine owner composition and independent worker supervision: deadline,
cancellation, parent death and descendant reaping, followed by installed resource
qualification. Fresh Engine/configuration integration, complete physical
consumers/claims, actual pool/share state, locks, effects, recovery and cutover
remain open. No Pi, Docker or block probe is part of this slice.
