# B1 UI mutation lifetime

Continue Stream B from main `7df3f7530865c31b7faefd1f7b12a25aa7655860`.
The central schedule registration worktree is already modifying Store, API and
status contracts. Leave it untouched. This slice fixes the existing B-owned
automation frontend before that registration; it introduces no route, timer,
grant, database, effect or shared shell change.

## Problem and choice

The read controller has generation checks and cancellation, but schedule create,
pause and notification acknowledgment run in component-local async callbacks.
Their `busy` value comes from a React render. Two invocations before another
render can both POST. Their continuations also outlive a principal, grant or
CSRF change. A successful read after a failed mutation must not erase the
mutation's outcome warning or imply that a lost POST response rolled back.

Keep the existing UI, endpoint bodies and server revision checks. Put mutations
under the same controller lifetime as reads. Component-local refs could prevent
duplicate clicks, but would leave two independent lifetime mechanisms and make
read/mutation races harder to exercise. A shared global mutation manager would
add cross-feature state without a need. The existing controller is the smallest
owner that can coordinate both.

## Behavior

The controller admits one synchronous mutation per section only after a fresh
read. It invalidates earlier reads before publishing pending state, then checks
its generation again before invoking the POST, including after synchronous
subscriber callbacks. Pending mutations retain their original abort signal.
Dispose, session expiry and React development cleanup invalidate continuations
and abort requests. Aborting a request does not prove server rollback.

Create/pause and acknowledgment use the current owner-scoped callbacks and CSRF
token. No POST is automatically retried. A lost reply leaves a visible warning
through subsequent successful reads; users must inspect committed state before
submitting another change. In particular, a paged schedule read cannot prove an
uncertain creation failed. The server still supplies all authorization and
durability; this controller adds neither authority nor an idempotency key.

HTTP session expiry clears all private state and invokes the shell once.
Forbidden/expired mutation authority clears the affected page and suspends its
controls until a new owner scope is mounted. Permission changes are part of
that scope. Ordinary mutation failures preserve feedback separately from read
errors. Read refresh and navigation cannot publish a pre-mutation page as fresh.

An inner panel keyed by principal, grant revision, CSRF and displayed capabilities
owns form state and controller. Browser layout cleanup disposes it during the
unmount commit; server rendering uses the passive effect equivalent. The outer
panel's public integration props stay unchanged.

## Validation and handoff

Execute the existing callbacks with a fixed render's `busy` value to reproduce
duplicate dispatch and an abandoned continuation before changing them. Add
deterministic controller tests for synchronous duplicate admission, reentrant
dispose, ignored abort, expiry, authority refusal, uncertain responses and read
fences. Exercise actual client fetch options to prove mutation abort propagation.
Run the full existing frontend cases, TypeScript and a production bundle using
pinned dependencies, with disk scratch outside `/tmp`.

These are local source and rendering checks. Browser event-loop, installed
API/UI, restart/deduplication, genuine observations and notifications remain with
the central registration and qualification owners. No B1/P04 or cutover closure
is implied.
