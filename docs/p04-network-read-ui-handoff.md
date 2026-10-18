# Cached network-group API/UI handoff

Stream B continuation on `engineer/p04-network-read-ui`, based on the egress
handoff `31b3b68` and combined main `7df3f75`. Tested product source is
`5e107872595176d0296d157bec445012ba834d15`. The
[design](plans/2026-10-17-network-read-ui-design.md) scopes this read surface;
the [packet](rewrite-evidence/p04/2026-10-17-network-read-ui/README.md) records
local checks. Production registration and browser/installed acceptance remain
pending. This does not complete the first durable scheduling milestone or P04.

## Supplied behavior

- `DeploymentBackend::network_inventory` defaults to unavailable. The proposed
  `GET /api/v1/deployment/networks` handler resolves a session before cache access
  and caps the serialized reply at 64 KiB. The actual backend must recheck current
  grants when reading the shared cache. The trait and tests are proposals, not a
  granted permission lease or shared Store implementation.
- `network-inventory-client.ts` constructs a bounded version-one view from the
  existing domain wire. It checks source/sample relationships, null counts,
  deterministic IDs/order, identity consistency, actual/hinted associations and
  health distinctions. It retains only selected fields; unknown environment,
  raw-response and other private fields are discarded. It is structural admission,
  not reconstruction of missing Engine facts or proof of collection provenance.
- Every fetch uses one fixed same-origin GET with no-store credentials policy,
  a 12-second abort deadline and a 64 KiB streamed body limit. UTF-8/JSON failures,
  overflow, missing bodies and aborted/truncated reads supply no report. Stalled
  streams are cancelled on abort. HTTP 401/403 take precedence over error bodies;
  other error bodies are not displayed. No arbitrary URL, probe or POST is added.
- `NetworkInventoryController` owns one principal/grant lifetime. Reads coalesce;
  disposal aborts and ignores late work. Reads, renders and expiry callbacks use
  the original inclusive 90-second Unix-second deadline, with early timer rearming
  and clock rollback checks. Expiry, failures and access loss clear retained rows.
  There is no polling or claim of continuous clock-change monitoring.
- `NetworkGroupsPanel` supplies providers, real member references, stopped
  containers, one-offs, configured health and unverified dependency hints through
  the existing dashboard styling. Unknown counts do not become zero; a fresh
  complete empty cohort can report no containers. Running providers without
  configured checks are labelled accordingly. Suggested providers never replace
  missing references. No lifecycle or probe control is offered.

The wire contains configured references, without provider image/start-time or
live namespace ownership. This panel does not join egress results by name or ID
alone. Neither a running provider, resolved reference nor an address difference
verifies actual shared networking, tunnel use or VPN safety.

## Central mounting requirements

1. Export the accepted domain/deployment/API leaves and implement the actual Core
   `DeploymentBackend`. Read the protected cached cohort built with `networks::
   observe` against the expected Engine. Recheck current grants, owner/lifetime,
   collection completeness and original time through the existing worker/Store
   contract. Cached shape validation cannot supply those guarantees.
2. Merge `deployment::routes()` before the existing shared API limits/security
   middleware and `with_state`. Preserve origin/session handling, no-store and
   security headers, header/body caps, deadlines and reader admission. Default
   unavailable behavior is required until the backend exists. The production
   root router was not changed by this slice.
3. Register approved schemas/generated types and replace the handwritten leaf
   DTOs. Mount one panel inside the authenticated dashboard:

   ```tsx
   import { NetworkGroupsPanel } from "./NetworkGroups";

   <NetworkGroupsPanel
     principal={session.principal.id}
     grantRevision={session.principal.grant_revision}
     expired={expired}
   />
   ```

   `expired` uses the existing shell expiry flow. Remove the panel on logout;
   principal/grant changes create a fresh controller. The panel refreshes cached
   data on focus/reconnect and explicit refresh, never by polling or probing.
4. Keep active probes separate, with explicit authorization and fixed approved
   transports under the protected worker's whole-collection budgets. GET cannot
   collect Engine/process/namespace facts or run an egress probe. The prior
   network/egress handoffs retain those requirements.
5. The existing qualification owner accepts actual API permissions/middleware,
   logout/grant-change/reconnect, deadline/budget failure, complete-empty versus
   failed/partial collections and browser rendering on the installed candidate.
   This slice changes no shared Store/schema/protocol, root manifests/lock,
   frontend shell or package/service activation and touches no production Pi.

## Local checks and limits

The combined Rust harness passes **461 cases**, including eight new network API
and wire tests (127 reader/host-policy, 27 API, 301 domain and six runner). Strict
all-target Clippy and Rustfmt pass. Root domain/executor libraries compile as
dependencies; their full unit suites are not credited. The same synthetic wire
file is checked against Rust `observe` output and consumed by frontend tests.

Pinned frontend type-checking and **123 cases** pass across eight files, including
34 new network cases. Coverage includes bounds, associations, cycles/self-reference,
raw-error/private-field refusal, exact expiry, clock rollback, coalescing,
abort/deadline cancellation and late results after disposal. These are local
controller/markup tests, not actual installed session lifecycle evidence.

A local synthetic fixture preview bundles successfully with pinned Vite/React
and the existing stylesheet. Visual inspection could not run: the browser tool
reported `Browser is not available: iab` and an empty browser inventory. The
temporary loopback preview server was stopped. Preview source, final build and
failed scratch-path/sandbox attempts are preserved; no screenshot, responsive
browser, visual or installed acceptance is claimed.

Initial diagnostics retain TypeScript's nullable-count narrowing error, a test
regex incorrectly matching the limitation phrase "VPN safety", and the isolated
Rust harness's missing explicit child-test module path. Final checks use the
corrected helper declaration, word-boundary assertion and explicit module path.
No registered scheduling, Store durability, installed recovery, ARM64, performance,
soak, defect closure or cutover gate is credited by this packet.
