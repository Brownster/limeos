# B1 integration handoff

Engineer B branch: `engineer/p04-automation-b1`, based on local main `0c7ec8c`. The [design](plans/2026-10-14-b1-automation-design.md) and [stream brief](plans/2026-10-14-engineer-p04-deployment-scheduling-runtime.md) define ownership. This is the observation-only leaf implementation. It is not registered B1 completion, persistent deduplication proof or installed qualification.

## Supplied leaves

| Path | Behavior |
| --- | --- |
| `crates/domain/src/automation/` | Closed operation definitions, UTC interval/window arithmetic, stable occurrence tuple, bounded views, frozen signal rules, threshold alert folding, delivery/retry/retention policy. |
| `bins/core/src/automation/` | One bounded scheduler page, typed observation job completion, bounded in-app publication. Effects can only create blocked records. Proposed adapter to the sole central Store. |
| `crates/api/src/automation.rs` | Six authenticated read/mutation routes using existing cookie/origin/session/CSRF/error helpers. Mutations require POST, typed input and expected revision. Responses cap at 64 KiB. |
| `frontend/src/AutomationView.tsx`, `NotificationsView.tsx`, `automation-client.ts` | Create observation schedule, UTC window, pause/resume, partial observations, active alerts, latest occurrence/job status, delivery failure/ambiguity and acknowledgment. Bounded response parsing; refresh after successful mutation. |
| `tests/fixtures/automation/` | Frozen arithmetic cases and an isolated compilation harness that adds proposed exports without changing shared crate roots. |

No shared source, Cargo manifest/lock, Store, SQL migration, resource registry, auth/protocol, package or dashboard file was edited. No `crates/persistence/src/automation/` implementation was invented: there was no accepted automation schema/Store contract on the starting branch. B will implement that leaf after the integrator supplies the frozen contract. The API trait and runner trait are explicit proposals for that handoff, not a parallel authority or transaction model.

## Central registrations

Small declaration/route changes for the integrator, against the starting roots:

```diff
--- a/crates/domain/src/lib.rs
+++ b/crates/domain/src/lib.rs
@@
+pub mod automation;
--- a/crates/api/src/lib.rs
+++ b/crates/api/src/lib.rs
@@
+pub mod automation;
@@
-pub fn router<B: Backend>(backend: B, origin: String) -> Router {
+pub fn router<B: automation::AutomationBackend>(backend: B, origin: String) -> Router {
@@
-        .fallback(|| async { failure(Error(ErrorCode::NotFound)) })
+        .merge(automation::routes::<B>())
+        .fallback(|| async { failure(Error(ErrorCode::NotFound)) })
--- a/bins/core/src/main.rs
+++ b/bins/core/src/main.rs
@@
+mod automation;
```

These snippets are intentionally not applied. The integrator implements `AutomationBackend` on the actual Core backend, registers the observation operation in the existing job machinery, implements `ObservationStore` using the central worker, and schedules/cancels bounded ticks under the existing supervisor. Mount routes before the existing `limits` middleware and `with_state`, preserving no-store/security headers, header cap and request deadline. Default trait methods return unavailable.

The current concrete backend is `Core` in `bins/core/src/main.rs`. Its production implementation must supply the six methods. The existing API-only test backends `Fake`, `Mutations` and `ReadBackend` also need empty `AutomationBackend` implementations to retain their unavailable defaults under the proposed router bound. This is a sequenced central registration change; B's isolated harness leaves the current router bound intact and compiles/tests the automation routes separately.

Frontend mounting: import the two views and `automationClient` into the existing dashboard. Read three separate pages, preserve independent cursors/partial/error state, and refresh after each mutation. Permission booleans must come from current authority; the server always rechecks. An unauthenticated response ends the session. Initial/reconnect reads must show committed state and last occurrence, never optimistic success. Keep pagination bounded rather than appending every page to memory. The handwritten leaf wire types are proposals; the integrator adds approved schemas/type exports and replaces these declarations with generated contract imports. Do not register effect operation adapters through this observation runner.

## Required Store contract

The following are requirements for central acceptance, not a proposed parallel SQL migration:

1. A schedule row carries an internal current owner/grant binding, bounded definition, paused flag, revision and settled watermark. `Schedule` exposes no credentials or caller-supplied owner. Revisions/sequences stay in `1..=9007199254740991`, timestamps in `0..=253402300799`. Creation uses the trusted clock; reject implausible caller anchors or replace them with that clock under an explicit contract. Pause/resume uses compare-and-swap and bumps revision; resume resets anchor/watermark to current time. Define the exact management/read/ack scopes centrally; never infer grants from a displayed role or a forged input actor.
2. Occurrences are unique on `(schedule, revision, due)`. Under the existing writer transaction, reload current definition/owner/grants/pause/revision, recompute the due decision using trusted time, reserve the tuple, create exactly one observation job and advance the watermark together. Existing tuple returns its original job/outcome. Skipped/blocked records create no executable job. Failed audit, missing authority, outbox/queue capacity or DB errors roll back all changes. Limit 128 schedules and a bounded queue; job/occurrence retention must preserve deduplication beyond deleted historical rows through the watermark.
3. A tick's queued list is advisory. Dispatch reads the actual queued job from Store, rechecks current authority and revision/pause, then uses the registered protected observation collector. Restart retries the same observation job or records its reconciled outcome; it never creates a replacement occurrence. Already-terminal completion is idempotent. An observation batch has at most 256 unique keys; missing/unavailable facts cannot mean healthy. Record partial completion visibly.
4. Completion consumes each durable observation sequence once, in order, and atomically folds alert material, inserts unique `(key, generation, incident/recovery)` events/outbox, transitions the job and audits. The model's `changed=false` allows unchanged alert material to remain unwritten. Sequence/job consumption still requires durable bookkeeping, so the leaf's simulated 24-hour test is not an idle-write measurement. Queue/outbox admission failure rolls back the transition. Scope resource keys internally to the correct observation owner/policy; two schedules must not collide through independently numbered sequences.
5. `announced` means an opening delivery has been reserved in the durable outbox; it does not mean an external receiver accepted it. In-app publication commits the readable row and delivered state together. External channel parity later needs ordered incident/recovery delivery: consult the actual opening delivery receipt, suppress/cancel a recovery for an opening never delivered, and preserve failure/ambiguous state. The frozen notifier's delivery-failure behavior must be covered in that central integration. Never mark a notification delivered before commit, and never automatically retry a non-idempotent ambiguous external send.
6. Cap outbox at 1,024 rows, attempts at four, explicit retry delays 30/120/600 seconds. Retain only delivered/acknowledged rows after seven days. Failed/pending/blocked/ambiguous rows remain actionable and count toward the cap. Acknowledgment is a current-principal-authorized POST with revision compare-and-swap, never a GET side effect. Failed/stale acknowledgment leaves the UI unacknowledged until a committed read proves otherwise.
7. All read pages are sorted, unique and limited to 32 rows, with opaque/stable continuation and explicit partial status; reads never update watermarks, retries, delivery, acknowledgment, expiry or retention. The Store ensures current owner/grants on every read. `last_occurrence` carries the committed tuple plus original job/state, skipped or blocked outcome. Snapshot consistency across page reads is central ownership.

Recurring effect policies, complete resource claims, fresh evidence, approval, storage locks and uncertain-effect recovery remain central/A/B follow-ups. A one-time human approval is never recurring authority. Backup/protection adapters consume A's accepted operations; deployment/APT adapters wait for B's registered effect milestones.

## Installed acceptance for the existing test owner

| Scenario | Required result |
| --- | --- |
| Create an observation schedule through API/UI with all model providers disabled | `201`, owned durable row, definition and UTC window visible after read/reconnect. Viewer/out-of-scope mutation fails. |
| Two tick workers at one due time, then restart and tick again | One unique occurrence and original job ID; no second job or notification. Repeat through separate real connections/processes. |
| Interrupt before/after reservation, completion and in-app publication commits | Whole transaction rolls back or returns the original committed row. Startup reconciles the existing job, with no duplicate alert/outbox event. |
| Queue/outbox/audit failure at each transaction boundary | No watermark, consumed observation or alert transition survives a failed admission/commit. Bounded error is visible. |
| Pause/resume and concurrent stale POST, then owner grants revoke/change/expire | Revision conflict/refusal; no replay of paused history; dispatch refuses changed authority. |
| Clock rollback/forward and UTC cross-midnight window boundaries | Stable tuple/watermark, explicit skipped/coalesced result, no work outside the current window. |
| Two failures, repeated failure, absent/unavailable observation, explicit recovery | One incident; no repeat transition; no false recovery on a collection gap; one appropriately delivered recovery. |
| Every effect operation scheduled before its adapter exists | Explicit blocked occurrence/status, zero effect jobs, executor calls or approvals consumed. |
| Failed/stale acknowledgment, external ambiguous outcome in a dedicated later channel scenario | Failure remains visible; acknowledgment changes only after commit; ambiguous send is not reported as success or blindly resent. |
| GET all reads and GET mutation paths | Reads produce no durable changes; mutation GETs return `405`. Wrong origin, session, CSRF and typed extra fields fail. Security headers remain present. |
| Installed UI pagination/reconnect/errors and 24-hour idle workload | Bound memory/queue and measure whole-service writes/CPU/memory against the architecture budget; no test-count substitute. |

The guest/hardware owner runs these once after registering the accepted Store/authority/API/UI. No Pi, production Docker, executor effect, install or service activation was performed by B.

## Remaining stream order

B1 awaits the central contract and registration before it can pass installed acceptance. B2–B4 then register stack discovery/review, whole-stack/catalog effects and every RW-042 catalog/media/tool inventory row. B5 provides fixed APT/prerequisite operations and safe version transitions. B6 binds schedules to accepted operations with separately reviewed recurring authority. B7 supplies integrated qualification and independent review. The dependency owner retains Cargo.lock/Rustls work; the existing test owner retains broad qualification.

UTC intervals deliberately replace process-local scheduling mechanics, but legacy local-time cron presets, deployment usage and ordinary notification settings/channels still require a checked inventory and migration explanation. The available P00 tracker leaves that inventory incomplete. Mattermost transport remains optional and is not silently removed. No P04/Pi cutover or defect closure is claimed by this handoff.
