# Register owned alert history and in-app notification access

Implemented by `f7d5946`. The
[handoff](../p04-b1-in-app-history-handoff.md) and
[local qualification](../rewrite-evidence/p04/2026-10-30-b1-in-app-history/README.md)
record the delivered boundary and remaining B1/cutover gates. The baseline below
describes the state before this implementation.

Baseline `039bf55` has the schema-12 Store's observation settlement, owned
alert/notification reads, in-app publication and acknowledgment. Core exposes
schedule metadata only. The proposed alert/notification routes and UI leaves
are not mounted. Register those existing Store operations next; collection,
configured policy and the production timer remain required before B1 completion.

## Authority and runtime

Add authenticated GET `/api/v1/automation/alerts` and
`/api/v1/automation/notifications`, plus POST
`/api/v1/automation/notifications/{id}/acknowledgment`. Use the existing API
middleware, Core backend and sole Database worker. Keep the current owner,
host-read and per-resource grant checks inside the Store's snapshot/transaction.
Acknowledgment retains current session/CSRF, delivered-row and revision checks.
No new grant, schema, independent queue, publisher or collector is introduced.

The API's original ten-second monotonic deadline applies before queueing,
inside the database worker and after its response. A timed-out/abandoned
acknowledgment may have committed; the UI reconciles through an owned GET and
never automatically repeats the POST. Original deadlines reach the existing
Store methods unchanged. Reads do not publish, acknowledge or change any
observation/occurrence watermark.

Pages retain at most 32 rows and 64 KiB. Alert cursors preserve literal UTF-8
keys up to 256 bytes; notification cursors and acknowledgment IDs are opaque
64-character IDs. Check outgoing row validity, order, cursor reciprocity and
response size at the API boundary. Missing observation streams remain partial;
an empty list must not claim health. Expose only in-app notification rows through
this registered slice. Disabled or missing external destinations do not gain a
sender by registering these endpoints.

## Contracts and UI

Generate schemas, OpenAPI and TypeScript from the actual alert/event/delivery,
notification and acknowledgment Rust types. Replace the proposed handwritten
client shapes. Preserve existing runtime decoders and their input bounds.

Mount a dedicated alert/notification panel in the authenticated dashboard,
alongside the existing schedule management panel. Reuse the bounded controller
with an alerts/notifications-only mode. Reset ownership on principal, grant
revision or CSRF change; abort and discard old reads/mutations on unmount.
Focus/online/manual refresh reads pages only. Show partial, stale, expired,
forbidden and unavailable states distinctly from an empty fresh page.

Show acknowledgment only for a delivered, unacknowledged row. The server
rechecks current ownership/resource grants; client role labels grant nothing.
Keep pending, failed and ambiguous delivery visible without a resend control.
Use the existing component styling and actual stored data. State plainly that
new health observations are unavailable while the genuine producer remains
unregistered; no synthetic incident or successful delivery is inserted to fill
the panel.

## Validation and remaining gates

Exercise the real Core database worker and authenticated API router with
file-backed SQLite. Seed history only through the genuine reservation/private
lease/settlement/publication methods with clearly labeled fixture health facts.
Cover owner isolation, current read-scope revocation, expired sessions, CSRF and
GET acknowledgment refusal, stale revisions, delivered-only acknowledgment,
lost replies and restart persistence. Check that GETs create no rows/events and
that invalid/oversized/out-of-order backend pages refuse at the route boundary.

Run affected Rust suites, strict Clippy, formatting, actual regenerated contract
drift, frontend checks and relevant controller/render/browser scenarios under
the existing repository-disk/one-job/3-GiB memory rules. Bind evidence to its
actual source; broad ARM64/package acceptance is not inherited.

This delivers registered history/acknowledgment, not complete B1. Configured
policy revisions and silence metadata, genuine fresh scoped collection,
resource replacement, bounded supervised scheduling/publication/retention,
installed concurrent-tick/restart/idle-write evidence and ordinary external
notification parity remain open. A1 normal-traffic capture/recovery, broader
A/B parity, P05 and P06 cutover requirements also remain open. No production Pi
access or publication is authorized by this design.
