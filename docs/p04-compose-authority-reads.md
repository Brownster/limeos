# Authenticated Compose review reads

`Store::resolve_compose_session` and `Store::resolve_compose_task` resolve the
ordinary preview/scopes needed by [retained review subjects](p04-compose-review-subjects.md)
through actual current Store authority. The session resolver also backs the existing
Core preview-read API, preserving its original `PlannedCompose` response. No new
route, executor request, job, approval, claim, lock or recovery operation is added.
Schema stays at 9; all migrations, contracts and dependencies remain unchanged.

Inputs are a session digest or task digest/transport peer UID/task ID, the trusted
current P03 catalog, an ordinary expected preview and its exact requirements.
Production chooses fixed current wall time; callers select no clock. The future
composition root supplies actual kernel peer credentials for task calls, as in P03.
These Store methods do not independently observe the transport or protected catalog
file; those duties remain with the composition root.

Use one explicitly deferred SQLite transaction for a consistent read snapshot.
Authenticate the actual session or task before processing caller project content.
Load the actual owned stored preview through existing cancellation, canonical body/
digest, revision, catalog/template/source/impact and current policy checks. Additionally
require stored revision/expiry columns to match the canonical plan. The API regression
demonstrates that inconsistent expiry metadata previously returned an ordinary preview
and now returns `state_not_durable`.

Compare the entire expected preview with the bounded stored value before serializing
or copying caller models. Derive requirements from the trusted normalized catalog;
the supplied one-or-two scopes must match exactly, including order. An omitted,
extra, duplicated, reordered or wildcard elevated requirement refuses. For a task,
recheck actual UID/task/generation/revision/expiry/revocation and every exact token
scope, with the same actual principal. Administrator role alone grants nothing;
media/container grants and wildcard elevated grants cannot satisfy deployment scope.

Recheck actual identity and stored-preview validity at the fixed end time, reject
wall rollback and require actual database generation to match the open Store.
The returned private `AuthenticatedComposePreview` retains only actual principal,
stored preview, exact requirements, generation/check time, identity kind and expiry.
Its borrowed serialize-only observation is counted against 64 KiB, keeping the
existing 48 KiB preview cap. Token/session digest, CSRF, password and approval are
absent. Owner/report cannot deserialize or provide a constructor. `into_preview`
returns ordinary preview data for the existing API, without an approval or lease.

This is a past read-only authority observation, without a writer transaction or
permission lease. Concurrent revocation after the snapshot is seen by the next
resolution. A previously successful value remains past data and cannot authorize
an effect. The future host-review broker must resolve authority before and after
genuine host validation; approval additionally needs current human session/origin/
CSRF and its own durable operation-bound path. Task tokens still cannot approve.

The borrowed observation's preview/scopes fit the retained subject's existing
`check_current` inputs. Persistence depends on domain/identity/policy, without an
executor dependency. Root transport/broker composition remains open; ordinary values
crossing that boundary cannot reconstruct retained descriptors or host evidence.
No fresh executable approval, acquired lock, write, shutdown or receipt is created.

Local acceptance uses actual temporary SQLite databases, session/task records, roles,
grants, previews and approvals. Resolution passes under `PRAGMA query_only=ON` and
preserves total changes, audit/events, generation, preview expiry/approval, jobs,
resources/locks and schema. Real concurrent WAL revocation preserves a consistent
read snapshot and fails on the next resolution. Restart preserves the session and
invalidates the task generation. See the [design](plans/2026-10-14-compose-authority-read-design.md)
and [acceptance packet](rewrite-evidence/p04/2026-10-14-compose-authority/README.md),
including complete failed diagnostics and the Core regression's red/green logs.

Build/cache/general scratch remains on disk; only existing precompiled storage
fixtures use small owned auto-cleaned tmpfs runtime. This database slice uses local
tests and needs no Pi access. The operator permits host testing after 22:00 Europe/
London; any needed run must preserve the frozen app/media/mounts and host health,
with the previous 07:00 return boundary. Cutover remains deferred. The signed ARM64
candidate stays frozen. Runtime/physical completeness, root broker, durable fresh
approval, acquired locks/recovery, shutdown/effect-time checks, worker integration,
independent review and public root/installed/resource/performance qualification remain
open. No P04 or cutover gate closes.
