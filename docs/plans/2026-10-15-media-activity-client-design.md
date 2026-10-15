# Media activity browser delivery

Stream B follow-up to `e1d8b04`. This adds the browser owner for the existing
cache-only activity API leaf and presentational view. It requires no Stream A
changes, persistence, shared registration or host access.

The client reads only `GET /api/v1/deployment/activity`, with same-origin
credentials, no browser cache, a 12-second deadline and a 64 KiB streamed body
limit before UTF-8 decoding or JSON parsing. Version-one admission validates
selected text, finite numbers, exact container/image identities, closed states,
provider families, service references, coverage, totals and original sample
times. Unknown fields are refused rather than retained in the private cache.
Errors use fixed client text; raw provider/server response text is never shown.
401/403 are handled before reading any body.

One controller belongs to one authenticated principal and grant revision. Mount,
focus, reconnect and manual refresh share one in-flight request. A one-shot timer
expires the original cohort at its eight-second boundary without polling; a
refresh does not extend old evidence. Failed refreshes retain visibly previous
rows with unknown totals. Access loss, logout, identity changes and unmount clear
the private cache and abort requests. Late replies cannot restore cleared data.
The panel must be mounted only inside the shared authenticated session boundary.

Freshness uses the server's integer-second contract: a sample is current through
`sampled_at + 8` and becomes previous at `sampled_at + 9`. The browser evaluates
its clock on refresh, render and expiry; a future server generation is unknown.
Background browser timer delays are covered by render/focus re-evaluation. No
permanent timer, recurring polling, service HTTP calls or write actions are added.

Verification uses the Rust serialization fixture, malformed and inconsistent
responses, chunked body/UTF-8 limits, deadlines, aborts, fake-clock expiry,
coalescing, access loss, obsolete responses and render assertions. Strict
TypeScript covers the leaf within the existing frontend. Rust is unchanged;
the preceding packet retains its separate Rust source identity and test results.

Integrator follow-up remains: register the generated contracts, authenticated API
route and genuine protected collector; mount this panel with the session/grant
identity and remove it on logout. Installed browser, ARM64, performance and broad
qualification remain with the existing qualification owner. This leaf does not
establish installed RW-042 parity or authorize a production Pi change.
