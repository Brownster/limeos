# B1 UI mutation lifetime handoff

Branch `engineer/p04-automation-ui-lifetime`, based on main
`7df3f7530865c31b7faefd1f7b12a25aa7655860`. Tested product source:
`e9cb979442a03c6ba978654bfa3193f78a348804`.
The [design](plans/2026-10-18-b1-ui-mutation-lifetime-design.md) and
[local evidence](rewrite-evidence/p04/2026-10-18-b1-ui-mutation-lifetime/README.md)
describe this frontend repair. It registers no automation runtime and does not
complete B1 or P04.

## Change

Create, pause and acknowledgment now run through `AutomationController.mutate`.
It admits one synchronous write per section after a fresh read. Repeated
callbacks from the same render cannot dispatch another POST while one is
pending. A schedule write and an acknowledgment can each be pending, for a
maximum of two admitted writes in this panel. Reads of a pending section wait
until the write settles; other sections can refresh independently.

The controller owns the write's abort signal and original lifetime. It checks
that lifetime before dispatch, after synchronous subscribers run and after
the response settles. Dispose and session expiry abort reads and writes and
invalidate their continuations. A transport that ignores abort cannot update a
new owner or expire its session. HTTP 401 clears private state and signals the
shell once. Forbidden/expired write authority clears the affected page and
suspends its controls for the rest of that owner scope; subsequent permitted
reads may display data without restoring mutation permission.

Mutation feedback is separate from read errors. A successful refresh therefore
preserves a revision-conflict message or an unconfirmed-outcome warning. The UI
never automatically repeats a POST or marks an acknowledgment optimistically.
After a lost response, inspect committed state before submitting another change.
A successful paged read cannot prove an uncertain creation rolled back. Closing
the panel aborts local work without making a server rollback claim; reopening
does not replay the POST. No idempotency key or durable client ledger is added.

`AutomationPanel` keeps its existing public props. Its inner owner is keyed by
principal, grant revision, CSRF and displayed management/acknowledgment
capabilities. Form state and pending feedback reset with that owner. Browser
layout cleanup disposes the controller during the unmount commit; SSR uses a
passive effect and starts no requests. This follows the pinned React 18.3.1
lifecycle; browser execution itself remains unrun.

`AutomationView` and `NotificationsView` are now controlled leaves. Their former
CSRF/changed/failed props are replaced with create/pause/acknowledge callbacks
and pending/error display props. Integrators should mount `AutomationPanel`,
which supplies those callbacks; direct leaf consumers must supply their own
controller owner. Missing mutation callbacks leave the relevant buttons disabled.
The client adds an optional abort argument while preserving existing two-argument
calls, fixed endpoints, request bodies, revisions, CSRF and response limits.

## Local validation

The old component callback was extracted with the pinned TypeScript AST and
executed with one render's `busy` value. Two invocations sent two actions and
both abandoned continuations invoked the completion callback. This reproduces
the source-level race; it is not a browser event test.

The final source passes 104 frontend cases across eight files, including 15 new
mutation regressions. TypeScript passes. The existing main production UI build
passes, and a separate production bundle explicitly imports the supplied panel
and includes the changed controller and view. Main still does not mount this
panel, so the main bundle alone would not verify its bundling.

The generic Node subprocess helper returned no API case output and was refused
by its count check. An explicit test-runner attempt reported only the file as
one case; it receives no four-case credit. The final API check transpiles the
unchanged actual API source with pinned TypeScript and runs its four original
assertions directly, preserving individual TAP results. All other files ran
directly with their actual case totals. Failed helper attempts are retained in
the packet. No Rust or installed suite was run for this frontend-only change.

## Central follow-up

The central schedule worktree was actively preparing management registration
while this slice was built; its files were left untouched. This branch does not
consume its uncommitted status shapes. Reconcile the accepted suspended/status
contract and generated wire types before mounting the panel. Preserve current
server authority checks and represent an undispatchable adapter honestly.

Store reservation/dispatch/completion, genuine protected observation lifetime
and cleanup, per-resource authority, alerts/outbox, notification publication,
acknowledgment, timer supervision and API/shell registration retain their
central handoffs. The qualification owner still needs installed browser,
permission/CSRF, concurrent tick, restart and failure acceptance. No package,
service, Pi, host effect or release was changed here. P05/P06 cutover gates and
existing resource/performance gates remain open.
