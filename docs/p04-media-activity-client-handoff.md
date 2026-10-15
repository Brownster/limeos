# Media activity browser client handoff

Engineer B, 15 October 2026. Branch `engineer/p04-media-activity-client`, base
`e1d8b04`. Tested source: `814e9a23e41186e56c162f7f3a4bde4fe5607091`.
This completes the browser ownership work left in the preceding
[activity handoff](p04-media-activity-handoff.md), without Stream A dependencies.

## Delivered

`frontend/src/media-activity-client.ts` reads the fixed authenticated cache
endpoint `GET /api/v1/deployment/activity`. It sends same-origin credentials,
disables browser caching and refuses redirects. A 12-second deadline covers
fetch and body consumption. A 64 KiB streamed-byte ceiling applies before fatal
UTF-8 decoding and JSON parsing; cancellation releases the body reader. Access
loss is recognized before reading any error body. Client errors use fixed text;
raw provider/server bodies, URLs and thrown error messages never enter the UI.

Version-one admission checks all expected fields, closed provider/state values,
exact full container/image IDs, calendar-valid Engine timestamps, UTF-8 string
bounds, numeric bounds, service references and provider families. It checks
coverage, omissions, visible counts and nullable totals against the Rust model.
Unrequested fields are refused, including extra fields inside display rows.
This guards display integrity; it provides no source ownership, authorization,
HTTP collection proof or effect authority. Standard JSON parsing is not a
signature/canonicalization or duplicate-key protocol.

`media-activity-controller.ts` coalesces mount/focus/reconnect/manual reads. It
owns one original-cohort expiry timer and never polls. Integer-second samples
remain current through `sampled_at + 8` and become previous at `+9`, matching the
Rust `now - sampled_at > 8` rule. Refreshing old data does not extend its lifetime.
Focus also checks age before joining a pending request, covering delayed browser
timers. A failed refresh retains previous rows with unknown totals. A genuinely
new sample can restore current data. Clock rollback remains unknown.

401 clears private rows and calls the session-expiry callback once. 403 clears
rows and disables refresh until the authenticated context changes. Disposal
aborts work, cancels timers and clears state; generation checks ignore obsolete
successes and errors even across a strict-effect restart. Selected media titles,
viewer names and devices remain private in-memory display data. The code adds
no local storage, logs, provider credential access or service HTTP calls.

`MediaActivityPanel.tsx` binds the controller to React and the shared session's
principal/grant revision, subscribes to focus/online events and cleans up on
context changes or unmount. An absent principal renders no panel. The existing
view receives the browser's integer clock and explicit loading/stale/error state;
access denial disables its refresh button. Existing dashboard styles are reused.

## Integration

1. Take this branch after its media activity/library/catalog/B2 ancestors. Other
   B branches retain separate handoffs; this branch does not aggregate them.
2. Register the preceding Rust/API leaves, generated contracts and genuine
   collector/cache through the integrator's shared path. The cache route remains
   default-unavailable until its backend is implemented. GET must launch no
   collection or credential read. Browser validation cannot repair missing
   collector identity, completeness or audience binding.
3. Mount `MediaActivityPanel` inside the authenticated shared shell, passing the
   actual current principal, grant revision and existing session-expiry handler.
   Remove it on logout, and change its identity props whenever either changes.
   Use the shared session boundary for revocation; do not retain an old panel
   after authentication is cleared. No central `Dashboard.tsx`, `api.ts`, API
   aggregator, schema, Store, Cargo manifest/lock or activation file changed here.
4. Have the existing qualification owner exercise the registered candidate:
   focus/reconnect/manual refresh, delayed reads, 401/403, logout and grant change
   while reads are pending, provider failures/partial queues, original expiry,
   and complete-empty versus unavailable discovery. Verify these in a browser
   with real collector permissions and provider versions before accepting parity.

The current slice progresses independently of A. A's existing dirty mount
integration worktree was read for coordination and left untouched. Durable B1
automation still needs the accepted central Store/transaction registration;
this client does not supply a second persistence path. Installed RW-042/RW-045
parity, the first registered B milestone and P04 closure remain pending.

## Evidence

[Packet](rewrite-evidence/p04/2026-10-15-media-activity-client/README.md): **39 passing
frontend cases** (29 client/controller/admission cases plus the existing ten
activity view cases), strict TypeScript and whitespace checks. The Rust-generated
wire fixture binds the proposed client to the prior serialization test; synthetic
mixed-provider cases exercise additional wire invariants. Rust is unchanged and
was not rerun; its preceding 107-test packet has its own source identity.

The initial exact-boundary test padded to 65,536 characters, yielding 65,537
UTF-8 bytes because of a middle dot. The client correctly rejected it. The
corrected fixture pads by encoded bytes and checks both accepted 65,536-byte and
rejected 65,537-byte bodies. The failed run and test source are retained.

The final script needed execution outside the sandbox after Node's child `git`
launch returned EPERM. All source-bound product checks passed. A later metadata
command reported spurious linked-package/dev-dependency problems through the
worktree's `node_modules` symlink. Running npm in the real dependency directory
passed after verifying both package manifests exactly match this worktree.
The failed report, correction and unchanged source checks remain in the packet;
no dependency installation or change was required. Node reports its expected
experimental MockTimers warning for the deadline tests.

These are local unit/fake-fetch/fake-clock/static React checks. They do not prove
browser event/lifecycle behavior, live collection, API registration, service
compatibility, installed packages, ARM64/Pi health, performance, soak or cutover
readiness. No host operations, push, merge or production changes were performed.
