# A1 registered mount UI lifetime repair

Tested product source: `7bdc36b4bd297ce80e4837c72ac96c07e9938ddd`.

Stream A assistance on `engineer/p04-a1-ui-lifetime`, based on main `7df3f75`.
This repair is wired through the existing `StorageMounts` component and existing
Dashboard import/props. It is not an unregistered replacement flow. The
[design](plans/2026-10-17-storage-mount-ui-lifetime-design.md) and
[evidence](rewrite-evidence/p04/2026-10-17-a1-ui-lifetime/README.md) describe its
scope. A1 installed mount/negative/interruption and browser acceptance remain
open; this does not complete A1, Stream A or P04.

## Reproduced defects and repair

The preserved baseline component contains two reproducible asynchronous races.
Its confirmation callback calls queue after an approval completes even if
`alive.current=false`. Its history callback permits an older read to erase a
newer job response. Local failing tests extract and execute those actual starting
callbacks with supplied fake APIs. They prove callback behavior, without claiming
a browser reproduction or performing an API/host effect.

`StorageMountController` now owns each CSRF session lifetime. Catalog and history
reads coalesce independently; generation checks and aborts prevent old responses
or errors from populating a restarted/abandoned owner. Queue/cancel start fences
earlier history reads before any POST. Completion refreshes committed history;
failed reads are explicitly unavailable rather than successful zero rows.
Synchronous pending guards stop duplicate click submissions independently of
React render timing. Every asynchronous continuation checks the original owner
before another POST, including the approval-to-queue boundary. State publication
is also checked for synchronous disposal before the initial POST. Read callers
retain the original promise even if a subscriber disposes the controller.

The exact proposal, approval and idempotency key survive an uncertain queue
response in memory. An explicit retry reuses the tuple, without new approval or
key. An already saved tuple can be retried after its approval deadline: the
existing server `replay_storage_mount` decides whether it was actually consumed
before applying fresh queue checks. Client tests use a supplied replay response;
they do not provide new DB durability proof. First submissions still refuse
expired/future/invalid local time. New explicit review replaces the draft; a
closed uncertain review can also be reopened using **Return to pending mount
request**, preserving its original tuple. Browser reload does not reconstruct an
in-memory tuple; current durable history remains the source of job state.

Transport abort/navigation never implies that a submitted server mutation was
canceled. The controller does not send cancellation automatically. Only an
explicit current queued-job cancel uses the existing cancel POST; cancellation
errors stay visible after subsequent history refresh. Unknown and
needs-intervention outcomes keep their existing reconciliation wording.

HTTP 401/403 from any operation clears targets, proposals, job rows and retry
tokens and prevents further work. Only 401 invokes shell expiry. Error text uses
fixed messages and bounded audit references, including explicit collection-limit
and protected-configuration failures, without raw proxy diagnostics. The outer
component keys a fresh child by the existing CSRF prop; callback identity changes
no longer reset its effects. The shell currently supplies no principal/grant
revision to this component. This is a session-lifetime repair, not client proof
of current grants; existing server checks remain mandatory. No parallel grant,
authority, lock or recovery registry is introduced.

## Storage-owned transport and presentation

The previous shared `request` helper replaces caller cancellation signals with
its timeout. The storage-owned client now composes both signals under a
12-second per-request deadline, preserving fixed route families, POST/CSRF
bodies and generated DTOs. It uses same-origin/no-store and refuses redirects.
Strict UTF-8/JSON admission caps outgoing JSON at 32 KiB, catalog/individual
plan/job responses at 32 KiB, approval/error bodies at 4 KiB and job history at
1 MiB. The history ceiling covers the existing maximum of 32 server plans, each
bounded to 24 KiB plus row overhead. Abort/overflow cancels a stalled body; HTTP
401/403 takes precedence over malformed body parsing. The shared API helper is
unchanged.

The controller checks counts, duplicate IDs, displayed device/plan fields and
job/action correlation before presentation. Those checks are not a complete Rust
schema validator and confer no physical/effect authority. Full proposal facts
remain the server's validated opaque intent; the UI does not rebuild owners,
normalize disk paths or invent an executor receipt.

The component consumes the controller through React's external-store API and
retains the existing review, focus containment, job controls and three-second
visible-page history cadence. Reads coalesce and stop after disposal/access
loss. Modal close/Escape is disabled during a bounded submission; it is not
labelled as job cancellation. Loading/error history no longer renders **No recent
mount jobs** as if a complete empty read succeeded.

## Ownership and acceptance handoff

Changed production paths are only `frontend/src/storage/{StorageMounts.tsx,
client.ts,controller.ts}`. Tests are new focused files under `frontend/tests/`.
Shared Dashboard/API/style, generated contracts, manifests/locks, Store/protocol,
executor/package and existing qualification harnesses remain unchanged. The active
`p04-mount-subphase-diagnostics` worktree's mount-admission and guest files were
left untouched. Its earlier `cb57e1a` baseline has identical storage frontend
preimages, so this UI repair does not depend on that diagnostic implementation.

The integrator can accept the repair commits onto the current A1 candidate
without a new route, schema or shell patch. The qualification owner should include
its separately owned `8c5b498` browser-fixture isolation correction when assembling
browser tests; do not overwrite that owner's harness with the older main fixture.
Run the mounted UI against the accepted source, specifically checking navigation
or session replacement during approval/queue, stale history across mutations,
lost queue reply/exact retry, close/reopen, cancel refusal, 401/403 and keyboard
focus/Strict Mode cleanup. Those actual browser cases were not run here.

Local validation passes **129 frontend tests**, including 40 new mount cases,
and the actual production UI build/type-check. Two baseline callback failures
and two intermediate reentrant-disposal failures remain with final passing
checks. The test syntax and reused-runner scratch-path mistakes are retained
separately. Tests use synthetic APIs/bytes and no server or disk effects. The
browser tool reported no apps/browsers; no visual review or screenshot ran.
No Cargo, guest, package, ARM64/Pi, performance, recovery, installed mount or
defect-closure result is credited. Bundle size/build duration are not performance
qualification. Remaining physical admission and crash/receipt gates retain the
existing Stream A/integrator/test-owner allocation.
