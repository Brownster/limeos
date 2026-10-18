# ObserveHealth schedule metadata

These fixed routes persist and display ObserveHealth schedules. Collection and
dispatch remain disabled. Creating or resuming metadata starts no timer, job,
reader, model, notification or executor effect.

```http
POST /api/v1/automation/schedules
Origin: https://configured-limeos-origin
Cookie: __Host-limeos=<current-session>
X-CSRF-Token: <current-session-csrf>
Content-Type: application/json

{"interval_seconds":60,"grace_seconds":10,"missed":"coalesce_latest","window":null}
```

Creation returns 201 and the owned schedule. Operation and owner are fixed by the
server; anchor is trusted transaction time. Input accepts only interval, grace,
missed-run policy and optional UTC window. The existing interval, grace and
window validator applies. Unknown fields, including paths, operation, owner,
anchor, readiness, grants and clocks, return typed InvalidInput/400.

Current stored `AutomationManage(automation:observe-health)` and
`HealthRead(host:system)` grants are both required for writes. A role label is
insufficient. An eligible Administrator can use the separately documented
password-reverified self-enablement endpoint, then log in again. Default grants
remain unchanged. Schedule metadata does not confer read authority over any
container or storage resource.

`GET /api/v1/automation/schedules?after=<opaque-id>` needs a current session and
host-read grant. It returns only the caller's ObserveHealth rows, sorted by ID,
with at most 32 rows and 64 KiB. The optional next cursor is the last returned ID.
Reads use one pinned database snapshot and do not audit, resume, rebind grants,
advance watermarks or trigger collection. Other operation kinds remain outside
these fixed routes.

Readiness is derived from stored facts and current authority:

- `awaiting_producer`: unpaused metadata has current management authority, but
  no genuine producer is registered. This is not running or healthy evidence.
- `authority_suspended`: an unpaused schedule's owner revision is old or current
  management permission is absent. A fresh valid login with read scope can see
  it; an invalid session or missing read scope remains an error.
- `paused`: metadata is paused. Reads never refresh its authority binding.

Pause or explicitly resume with
`POST /api/v1/automation/schedules/{id}/pause` and
`{"revision":1,"paused":true}` or `paused:false`. Current Origin, session,
CSRF and both grants are required. The expected schedule revision is checked
first; stale retries return Conflict/409. A no-op requires the same paused state
and owner binding. An accepted change increments the schedule revision and
records the existing audit atomically. Only resume refreshes the owner grant
binding and resets the server anchor/watermark. Pausing preserves the anchor;
it cancels only that schedule's queued observation jobs and retains unrelated
jobs and effect claims. No automation job can start under the unchanged schema 11
guards.

Creation intentionally has no automatic POST retry or definition-based adoption:
repeating it creates a separate schedule. A lost response must be reconciled
through the owned list. For pause/resume, read the committed state after an
ambiguous response before choosing a new revision. A transport timeout alone
does not prove rollback.

The original ten-second monotonic budget spans body parsing, database queueing
and transaction checks. Final write authentication and trusted wall time are
sampled after acquiring IMMEDIATE; reads pin their snapshot before sampling
time. Detected precommit expiry, audit admission or SQL failures roll back the
whole change. The existing 128-schedule cap and bounded audit storage remain.

The isolated hosted [bootstrap run](https://github.com/Brownster/limeos/actions/runs/39155342674)
passed on source `d962272ed73970a3de2b72b612161dd4836dfe3d`, whose 15 Rust inputs
match this slice. It built the real password worker and passed 344 affected
tests: 17 API, 27 Core, 172 domain and 128 Store, with none failed or ignored.
Scoped all-target strict Clippy, formatting and repository checks passed. The
actual Rust generator emitted 64 files; this assembly copies its six changed
outputs byte for byte: four schedule schemas, OpenAPI and TypeScript types.
The downloaded artifact's API/ZIP digest, 78 checksums and 19 before/after Git
input bindings were verified. This is hosted AMD64 source qualification, with
no installed service or collector execution.

The initial local compiler failure and subsequent owned memory-guard
termination remain recorded; neither ran tests. The separate ordinary
bootstrap CI intentionally sees old tracked contracts; its outcome is preserved
independently. TypeScript checking passed on the assembled declarations using
the existing identical locked dependency tree, and repository checks passed.
Final assembled-source contract consistency and full normal CI remain pending,
along with installed/ARM64 and genuine
producer/lifetime/cleanup/resource/cutover qualification. No runtime or cutover
gate closes here.

Estimate: eight human hours, scope review at twelve; P04 stays 320/480. Measured
human effort is unknown.
