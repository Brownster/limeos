# Register fair observation supervision through Core and the UI

Baseline `ae60d7d` admits real polls and original queued jobs before protected
reads, but no periodic producer invokes them. Register one owned Core future
that rotates work fairly, publishes existing in-app events and reports partial
or unavailable state through authenticated schedule views. This delivers actual
automatic observation behavior while preserving the remaining B1/P04 and full
cutover gates.

## Fair bounded work

Add query-only internal Store hints for a sorted page of at most 32 schedule
owners and one work item for an owner. Existing oldest queued observation work
takes precedence for that owner. Otherwise select the next schedule after that
owner's volatile cursor, wrapping at its end. Keep the actual MAX_SCHEDULES=128,
MAX_TICK_SCHEDULES=32 and existing queue/lease bounds. Hints restore no authority;
all admission, recovery and completion still use current Store checks.

Maintain an owner-page cursor and one schedule cursor per owner, each bounded by
128 entries. Advance a schedule cursor when its candidate is examined, including
paused/not-due/refused candidates. A single sorted schedule-page scan can starve
later schedules belonging to the same owner because owner single flight always
selects that page's first row. Rotating per owner avoids that failure. One busy
or invalid owner must not block other owners or notification maintenance.

Ordinary observation candidates use transition-only poll admission, never the
old durable reservation-per-sample path. Recover only already queued identities.
Unsupported effect candidates use their existing skipped/blocked settlement;
no effect adapter is enabled. Current grants, policy/schedule revisions, due
windows, missed-run policy and oldest-job ordering remain authoritative.

## One fresh acquisition, separate authorized transactions

Admit at most one read per owner, up to 32 per pass. With no eligible permits,
contact no reader. Share one genuine fresh cohort across the admitted owners,
using the existing Core/storage semaphore and protected health RPC. The window
starts before that RPC, is at most four seconds and is bounded by the earliest
original permit/pass deadline. Each owner's completion binds all actual full-ID
members and rechecks current authority in its own transaction before that same
original read deadline. A copied shared report cannot renew freshness.

One fresh read per schedule wastes complete Engine enumeration and needlessly
reduces capacity. Reusing a cached report would weaken lifetime and replay
rules. Sharing one live original owner within a bounded pass preserves freshness
and complete scope checks without filtering unauthorized members. Earlier
owner commits can succeed before a later refusal or lost reply; their durable
identities remain authoritative and no cross-owner atomicity is promised.

Bound a pass to eight seconds. Use one serial Core future with a one-second
interval and skipped missed timer ticks; no overlapping pass or catch-up burst.
Publish one 32-row in-app page and prune one existing retention page independently
of root collection, preserving their fair cursors and current permission/policy
checks. Stable/no-work passes perform no new durable writes. Source/refusal
failures release matching permits with one 100-ms best-effort cleanup; cancelled
work keeps only original volatile expiry. Unexpected durable corruption stops
the worker/Core rather than being reported as healthy. No detached producer or
new thread/service/resource ceiling is introduced.

## Truthful current registration and last-attempt status

Add a scheduled observation status and an optional bounded runtime view with
phase (waiting, checking, last partial check, unavailable) and an optional valid
attempt time. Core projects it only after the Store authenticates the owned
schedule and only for current unpaused observation authority. Other owners,
suspended/paused schedules and unsupported effects receive no runtime details.
Store-only/no-running-producer views retain awaiting-producer semantics.

Keep an in-memory map of at most 128 diagnostic entries, bound to schedule
revision. A lifetime guard marks the producer active while its owned future is
running and clears that projection on cancellation/exit; duplicate runs refuse.
Diagnostics grant no authority, survive no restart and contain no paths, raw
errors or container counts. A bounded eviction loses only diagnostics. The
schedule page stays explicitly partial whenever automatic container checks are
registered. A completed partial check is historical, never a healthy/freshness
claim. Existing alert/notification views retain their original authority and
partial-state checks.

Update generated schemas/TypeScript and the existing frontend schedule table
to validate/render these states. Show automatic checks, last-attempt time and
unavailable/partial coverage plainly. Keep initial failure, stale-data controls,
ownership, permissions, CSRF and read-only GET behavior. Root reader policy and
services remain operator-controlled/dormant; missing setup or concrete grants
must be visible as unavailable, not silently granted or auto-enabled.

## Qualification

Use real Store/owned RPC fixtures to prove owner and same-owner schedule
rotation, one acquisition for multiple owners, empty/not-due/paused zero reads,
current grants/revisions, original queue priority/restart deduplication, partial
completion, source failure/retry, cancellation/single flight and bounded maps.
Measure actual stable runtime passes for zero SQLite/WAL changes and preserve
the distinction from an installed 24-hour soak. Verify API ownership, runtime
registration/stop projection and current frontend validation/status rendering;
inspect desktop/mobile fixture UI. Run workspace/opt-in Rust, strict Clippy,
actual schema generation/drift, frontend tests/build and repository checks.

Preserve source-bound attempts and original process identities. Repository-disk
scratch only, one Cargo job/test thread, 4-GiB starts and the unchanged sampled
3-GiB floor. No Pi, real Docker, package/service installation, external delivery
or push. Remaining host/SMART/mount/protection health, reviewed replacement/
retirement, safe history compaction, ordinary external channels, installed
root-positive/API/UI/restart/ARM64/package/return/footprint/soak and A1/B1/P04/
P05/P06 acceptance remain required. The existing inventory-performance failure
and policy/pre-schema-14 return requirements remain open.
