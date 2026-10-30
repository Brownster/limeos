# B1 owned alert and in-app notification history

Source `f7d5946` implements the
[registered-history design](plans/2026-10-30-b1-in-app-api-ui-design.md).
[Source-bound local evidence](rewrite-evidence/p04/2026-10-30-b1-in-app-history/README.md)
qualifies the actual Core/Store/API integration and a separately scoped browser
fixture. This extends the [asynchronous Store bridge](p04-b1-async-store-bridge-handoff.md).

## Registered behavior

The authenticated router now serves GET `/api/v1/automation/alerts`, GET
`/api/v1/automation/notifications` and POST
`/api/v1/automation/notifications/{id}/acknowledgment`. Core dispatches these
through the existing sole Database worker. No database migration, independent
queue, collector, publisher, sender or timer is introduced.

Current sessions, owner identity and host/per-resource grants are checked inside
the existing Store operations. Acknowledgment additionally requires CSRF, a
delivered in-app row and a matching revision. Explicit replay is idempotent;
stale revisions refuse. Reads create no observation, delivery or audit events.
The original ten-second API deadline applies before queueing, inside the worker
and after the response. An abandoned or late acknowledgment may have committed;
its caller must reconcile stored state rather than infer rollback.

Pages contain at most 32 rows and 64 KiB. The API checks domain validity, ordering
and cursor reciprocity before serialization. Alert keys retain their literal
UTF-8 representation; notification IDs remain opaque 64-character IDs. Invalid
queries and unsupported output fail without admitting additional authority.
Schemas, OpenAPI and TypeScript derive from the actual Rust history types.

The authenticated dashboard opens a separate history panel on request. Refresh,
focus and online events read history only. Principal, grant revision or CSRF
changes dispose the previous owner and fence late replies. Missing, partial,
stale, forbidden and unavailable data remain distinct from an empty fresh page.
The panel states that new health observations remain unavailable.

Only delivered unacknowledged notifications expose acknowledgment. A lost POST
reply triggers GET reconciliation without an automatic second POST. Pending,
failed and ambiguous delivery remain visible without a send control. Literal
keys and summaries render as text. On narrow screens the labeled notification
table scrolls horizontally without overflowing the whole page.

## Local qualification

The final affected Rust run passes 617 runtime tests and seven compile-fail
doctests. Four new Core tests use real SQLite, the production worker and actual
authenticated TCP HTTP requests; three new API tests enforce route/output
boundaries. Strict Clippy, regenerated contracts, formatting, repository hygiene
and scoped Ruff pass. All 235 frontend tests and the checked production build
pass. The closed Playwright fixture verifies the exact built UI assets, owner
disposal, literal paging, acknowledgment/lost replies and desktop/mobile layout.

The evidence retains failed drafts and their distinct source identities. Four
final guard input maps match all 699 committed code inputs. The frontend build's
125 inputs and three UI asset hashes match the final browser's served bundle.
This is local integration evidence, not installed-package or ARM64 acceptance.
Repository-disk scratch, one-job compilation and the unchanged 3-GiB memory floor
apply. Holly's Pi was untouched and nothing was pushed.

## Remaining integration

Bind current configured policy revisions, complete frozen silence metadata and
resolve stable alert keys against concrete resource replacement. Default Store
policy is not migrated configuration parity; the
[frozen inventory](p04-frozen-notification-inventory.md) records the gaps.

Connect a genuine protected fresh collector with restart-policy, healthcheck,
SMART, required-mount and protection facts. Overview cache values cannot stand
in for these facts. Supervise bounded reservation, collection, publication and
retention with fair cursors and restart recovery. Schedule metadata continues
to report awaiting producer.

Then qualify the installed API/UI path with genuine production observations,
concurrent ticks, restart deduplication, current resource/grant changes and
24-hour idle writes. Ordinary configured external notification parity remains
open. This slice does not complete B1 or P04; A1 reliability, broader A/B parity,
P05's minimal assistant and P06's migration/return rehearsal remain required
before Holly's frozen Python app can be replaced.
