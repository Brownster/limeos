# B1 delivery ordering and UI controller

Continue Engineer B's observation-only leaves from `b639de3`, consuming the published dependency fix from main. The integrator still owns the automation Store/schema, current authority, job/runtime registration, route aggregation and dashboard mounting. No accepted automation contract is present on main at the start of this slice.

## Delivery ordering

Add a pure delivery decision over existing notification rows. A recovery can proceed only after a matching incident for the same alert key, incident generation, alert kind and delivery channel has a verified delivered result. A pending/in-flight/retrying opening means wait; exhausted failure or blocked opening keeps the recovery blocked; an ambiguous opening requires reconciliation. A missing opening receipt is incomplete evidence, not proof that delivery never happened. Reject mismatched receipts and clock/order corruption. The central Store must load owner-scoped rows and apply the decision and revision checks under its own transaction; this helper supplies no authority or receipt authenticity.

Only in-app publication is available in B1. The decision explicitly blocks external channels even when their retry policy is defined, until a channel is registered in a later accepted milestone. Restarted in-flight attempts become ambiguous under the same delivery identity; no replacement notification or implicit resend is allowed. Preserve delivered/acknowledged records for seven days after the later resolving timestamp.

Alternatives: marking every recovery delivered when queued misrepresents failed incident delivery. Guessing non-delivery from a missing/retained row can suppress a valid recovery. Both are rejected in favor of explicit waiting/blocking/reconciliation decisions and the original durable identity.

## UI controller

Supply a dedicated controller plus a mount-ready panel within B's UI leaves. Maintain one bounded page each of schedules, alerts and notifications, independent cursors/errors/partial state, and no accumulated history. Refresh after successful mutations, focus/reconnect and an explicit refresh; a failed refresh preserves prior data as stale and disables mutations until the current read succeeds. Initial failure is visibly unavailable rather than an empty list.

Serialize refresh batches, coalesce repeated refreshes, and abort old requests when changing a page or disposing the controller. Reject responses from abandoned requests so a delayed old page cannot overwrite a newer page. An unauthenticated response clears all private state, cancels pending work and calls the existing session-expiry callback once. The dashboard remains responsible for mounting the panel with current permission booleans and CSRF token. No new session store, credentials, polling loop or grants model is introduced.

Focused checks cover actual client/controller races with controlled responses, refresh/partial/failure/expiry behavior, page replacement, failed mutations, and delivery ordering/interruption/retention. API/domain tests remain scoped; broad installed/ARM64 qualification belongs to the existing test owner. UI browser checks may use a local preview with fixture responses and must be identified as such. No production-host testing or shared registration is performed by this slice.
