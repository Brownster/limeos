# Media activity read handoff

Engineer B, 15 October 2026. Branch `engineer/p04-media-activity`, base library
planning `1f64979`. Rust/API commit `80cf363`; final tested Rust/UI source
`873938a17b153415196874b8fa2d99a6c8d732cc`.

## Delivered leaves

- `domain::deployment::activity`: explicit Jellyfin/Emby, SABnzbd, Transmission
  and Sonarr/Radarr/Lidarr/Readarr providers; scoped container-instance samples,
  original timestamps, deterministic snapshot ordering, nullable totals, fixed
  failures and streaming output admission. These are ordinary diagnostic DTOs,
  never protected source owners, authority or effect receipts.
- `executor-container::deployment::activity::decode`: supplied-response bytes
  only. Preserve stream pause/transcode/tick progress and display metadata,
  SABnzbd numeric strings/byte conversions, Transmission download/seeding metrics
  and Servarr queue problem rules. Selective object-only decoding skips tokens,
  file paths, URLs and arbitrary unknown payloads. Selected titles/users/devices
  intentionally remain private display data; do not log them or raw errors.
- Deployment API leaf: authenticated **GET `/api/v1/deployment/activity`**, resolving
  a session before reading the backend cache and checking the 64 KiB response
  ceiling. POST is refused. The default backend is unavailable. GET launches no
  inspection, HTTP poll, credential read, write or job. This route is not merged
  into the production API aggregator by the branch.
- `MediaActivityView.tsx` and proposed leaf DTOs: a presentational read-only panel
  using existing dashboard styles, explicit unknown/partial/previous states,
  service failures, progress, observed queue counts and omitted-row counts.
  It receives a validated snapshot, advancing `now`, and a refresh callback;
  parent cache fetching, runtime decoding, deadline/session lifecycle and shared
  shell mounting remain unimplemented integration work.

Preserve frozen response semantics while correcting misleading empty/aggregate
results. Only complete fresh empty discovery produces zeros. Missing or failed
services do not hide successful peers; aggregate totals are null whenever
discovery/provider coverage is incomplete. A partial queue displays server total,
observed rows and problems in that page, never a whole-queue problem count.
Unknown metrics remain null, even on otherwise complete observations. Transmission
transfer rate includes the upload rate for seeding, matching the frozen display;
it is not a host network measurement. SABnzbd's aggregate rate is counted once;
when multiple slots say downloading, additional slot rates remain unknown.

Require the exact container resource, image, original start time, running flag,
name and provider association. Refuse duplicate/unrequested samples. Changed
identity, stopped services, stale/future samples, credential failure, timeout,
invalid response and unsupported data remain explicit. Expiry uses the original
eight-second cohort bound; no sample is refreshed by serialization or delivery.
Display at most 12 streams/transfers, retaining full admitted counts and explicit
omissions. Queue details show at most three of the inspected records.

Bounds: 32 KiB raw body before parsing; 24 rows per provider; eight transcode
reasons; eight targets/samples; 32 KiB aggregate selected text before clones;
64 KiB streaming snapshot. Selected display strings use UTF-8-safe byte bounds.
Numerics must be finite, nonnegative and within the JavaScript exact-integer
ceiling; percentages are bounded. Per-body caps do not qualify an eight-reader
worker budget or prove acquisition completeness.

## Required integration

1. Export activity alongside the accepted B2/catalog/library modules and merge
   deployment routes under the existing API limits/security/no-store middleware.
   Shared roots, Cargo manifests/production lock, schemas/generated contracts,
   Store, protocol and service activation remain unchanged. Other B branches
   (automation, network groups, images and packages) retain separate handoffs.
2. Supply an approved complete provider inventory and genuine current Engine/
   configuration identity, with original lifetime/revalidation. Do not copy the
   frozen name/image substring match, arbitrary published-port fallback or raw
   caller `/config` path into a credential/endpoint authority decision. Account
   for exact sidecar/network namespace and service/stack identity. A supplies any
   required physical evidence through the shared path; this slice needed no A
   edits and the dirty A mount work remains intact.
3. Implement a bounded credential-scoped transport/collector and common cache.
   Enforce approved endpoints/parameters, actual status/success, credential scope,
   redirects, body/time/concurrency/cohort budgets and original sample identity.
   Parsing bytes cannot establish any of these facts. Do not log API keys,
   query URLs, request/session tokens or response bodies. Credential discovery,
   TLS/authentication policy and actual service compatibility remain pending.
4. Current Transmission decoding supports the frozen legacy object-form response
   and requires explicit `result: success`; newer protocol shapes are unsupported.
   Its read-only torrent-get POST/session-ID handshake belongs to transport, not
   a caller-supplied arbitrary RPC method. SABnzbd completeness needs start zero
   and matching total count; missing metadata stays partial. Collect unfiltered
   pages deliberately: body counts cannot prove the original request scope.
5. Implement `DeploymentBackend::media_activity` on Core using the common cache,
   current token/principal/grants and allowed audience. Authentication alone is
   not the final authorization policy. Resolve and refresh data outside GET under
   the existing supervisor; GET must never trigger collection. No observation
   report may authorize an effect or feed a trusted alert without that binding.
6. Register generated DTOs and bounded wire admission/client reads, then mount
   the panel. Supply a one-shot expiry rerender with no polling loop; refresh on
   explicit request/focus/reconnect through the parent controller. Clear private
   state on logout, expiry, grant/principal changes and 401/403; abort/ignore old
   requests. The view's `now` comparison does not schedule its own rerender or
   replace this lifecycle. Unknown/partial/stale values cannot look idle/current.
7. Qualify real credential layouts, provider versions, sidecar ports, disconnects,
   pagination, source replacement and API/UI permissions on the registered
   candidate. Existing media seeding, quickstart, activity collection and complete
   RW-042 acceptance remain open; no feature is retired here.

Protocol references: [Transmission 4.0.6 RPC](https://github.com/transmission/transmission/blob/4.0.6/docs/rpc-spec.md)
defines explicit success and the session-ID handshake; [SABnzbd API](https://sabnzbd.org/wiki/configuration/5.1/api)
defines queue pagination and total/start metadata. These references supplement
the frozen fixture ports; they do not qualify installed current services.

## Evidence and limits

[Packet](rewrite-evidence/p04/2026-10-15-media-activity/README.md): **107 Rust tests**
(15 activity domain, 16 activity decoder, two activity API, plus 74 existing
library/catalog/B2/API regressions); **10 static React/expiry cases**; TypeScript,
strict Clippy, Rustfmt and harness Ruff checks pass. Wire serialization is checked
against the committed Rust-generated fixture. Response fixtures are synthetic
ports of frozen activity_service.py/tests; Python is read/hashed, never executed.

The UI regression reproduces a partial snapshot rendering supplied non-null
totals; the final view additionally requires `complete` before displaying them.
Initial harness diagnostics retain missing Tokio test-util and an omitted
non-test route wrapper; both are fixed in the final harness. No product lint is
suppressed. Helper formatting leaves all final harness bytes/lock unchanged.

No live media/Engine/credential/network collection, host effect, browser/layout,
installed/package/ARM64/Pi, whole-worker footprint, performance/soak or cutover
qualification ran. Static SSR is not browser interaction or installed UI proof.
Production registration, the first registered B milestone and P04 remain pending.
