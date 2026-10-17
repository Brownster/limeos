# A1 mount UI lifetime and replay repair

Help Stream A's registered `storage.mount@1` flow on main `7df3f75` without
touching active mount-admission/guest diagnostics. Own only
`frontend/src/storage/`, new focused frontend tests and narrow documentation.
Shared API helpers, shell, generated contracts, Store/protocol, executor and
qualification-owner harness files remain unchanged.

Two deterministic local regressions execute callbacks extracted from the actual
starting component. An approval completing after `alive.current=false` still
calls queue once. Two overlapping jobs reads let the older reply erase the newer
job. The current shared request helper also replaces caller cancellation signals
with its timeout. A boolean alive flag cannot distinguish abandoned generations.

Move the component's orchestration into a dedicated controller. Scope one owner
to the existing CSRF session value, so a changed value mounts a fresh child before
displaying previous state. Keep expired callback identity stable through a ref.
The shell currently supplies no principal/grant revision to this component;
server authorization remains mandatory, and this repair does not claim a new
grant-lifetime API. Current grant loss from any request clears private state and
stops work; 401 invokes shell expiry while 403 remains visibly denied.

Coalesce catalog/jobs reads, track original controller/read generations and abort
on disposal. Mutations invalidate earlier job reads before sending, then refresh
committed history. Check the active lifetime after every await before another
POST, particularly approval-to-queue. Late replies/errors cannot update a new
owner. Navigation cancellation does not prove that an already submitted server
mutation was canceled. Only the existing explicit queued-job cancel POST requests
that operation; unknown/needs-intervention jobs retain their existing UI meaning.

Keep the exact proposal, approval and random idempotency key after a lost queue
reply. Retry sends that same tuple without requesting new approval, including
after the review expiry when the server can return an already consumed replay.
There is no automatic mutation retry. Starting a new explicit review replaces
the draft. Prevent duplicate in-flight confirmations/cancels independently of
React render timing. Disable modal dismissal during a bounded submission; do not
describe closing or aborting transport as canceling a durable job.

Implement cancellation in the storage-owned client rather than altering the
shared helper. Preserve fixed route families, POST/CSRF bodies and typed generated
DTOs. Compose caller cancellation with a 12-second deadline, same-origin/no-store
and refused redirects. Bound requests to 32 KiB; catalog to 32 KiB, individual
plan/job responses to 32 KiB, approval to 4 KiB and history to 1 MiB (at most 32
plans, each server-bounded to 24 KiB). Decode strict UTF-8/JSON. HTTP 401/403 must
win over malformed bodies. These bounds do not replace server schema validation
or create physical/effect authority.

The component remains wired through its existing export and props; it consumes
the controller through React's external-store API. Preserve plan identity review,
modal focus behavior, current controls and job uncertainty text. Existing periodic
history reads coalesce and stop after access loss/unmount; no extra polling is
introduced. Tests use deterministic delayed API calls and streamed responses,
plus static markup and owner-key checks. Run pinned type checks, frontend tests
and a production UI build sequentially on disk. Browser/installed mount and
recovery acceptance remain with the existing qualification owner.
