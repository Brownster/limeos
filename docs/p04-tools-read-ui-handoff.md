# Cached CopyParty status UI handoff

Tested product source: `1dbcdda81a2d6d119d5b59ba762a389e17c0fc1b`.

Stream B continuation on `engineer/p04-tools-read-ui`, based on tools planning
`f8aec4a`. The [design](plans/2026-10-17-tools-read-ui-design.md) and
[local evidence](rewrite-evidence/p04/2026-10-17-tools-read-ui/README.md) cover a
mount-ready read panel. The existing Rust/API tools handoff retains its runtime
and effect requirements. Production mounting and installed/browser acceptance
remain open. This does not complete RW-042, durable B1 scheduling or P04.

## Supplied behavior

- `tools-status-client.ts` constructs a version-one view from selected fields of
  the existing Rust wire. It checks exact enums, time/sample relationships,
  null values for withheld components, literal path/port bounds and missing-file
  defaults. Unknown arguments, environment, diagnostics and links are discarded.
  Shape checks cannot establish protected provenance or current authority.
- `readToolsStatus` calls only the fixed
  `/api/v1/deployment/tools/copyparty` GET with same-origin credentials, no-store,
  refused redirects, a 12-second abort deadline and an 8-KiB streamed body cap.
  UTF-8/JSON/shape errors, missing bodies, overflow and incomplete/aborted reads
  yield no view. Aborting cancels a stalled reader. HTTP 401/403 takes precedence
  over malformed/private error bodies. Other error bodies are discarded; the
  controller displays fixed messages rather than raw transport errors.
- `ToolsStatusController` scopes one principal/grant/host lifetime, coalesces
  reads and aborts/ignores abandoned work. Read and expiry callbacks remove only
  component values older than their original inclusive 90-second deadlines.
  The earliest remaining deadline arms one local timer; early callbacks rearm
  the original deadline. No timer collects or refreshes. Already withheld
  components cannot become fresh just because time passes.
- Invalid clocks or rollback before report generation clear controller data when
  checked on reads/expiry callbacks. The view also checks time during rendering
  and immediately withholds unusable values. No continuous clock-change detector
  or synchronous state invalidation on every render is claimed. Read failures and
  HTTP access loss clear the whole retained report. Access loss stops further
  reads; 401 invokes shell expiry, while 403 remains a denied state.
- `ToolsStatus` shows separate configuration, legacy payload and service
  observations using existing dashboard styles and unique accessible headings.
  It distinguishes missing-file defaults, saved settings, unknown/read errors,
  absent versus unverified present payload, failed/transitioning/masked/missing
  services and original times. Equal saved fields or service activity never mean
  verified loaded configuration, trusted installation, readiness or file access.
  Only cached refresh is offered; no service URL or effect control is generated.

## Central mounting requirements

1. Complete the protected collection, cache/Core backend, grants, route middleware
   and schema/generated-contract registration in
   [the tools planning handoff](p04-tools-planning-handoff.md). This frontend
   increment changes no Rust source, wire fixture, shared shell, package manifest
   or dependency lock. Replace handwritten leaf types with approved generated
   types when the integrator registers them.
2. Supply the panel with the current principal, grant revision and opaque
   host/boot/cache observation lifetime from the accepted central context:

   ```tsx
   import { ToolsStatusPanel } from "./ToolsStatus";

   <ToolsStatusPanel
     principal={session.principal.id}
     grantRevision={session.principal.grant_revision}
     hostLifetime={currentHostObservationLifetime ?? null}
     expired={expired}
   />
   ```

   `currentHostObservationLifetime` is a central integration requirement, not a
   newly added session field or an identifier to fabricate in the browser. It
   must change on host, boot or protected-cache lifetime changes. Missing owner
   context withholds the panel and launches no request. Owner changes key a new
   mounted controller before it can display previous values; cleanup cancels old
   work. Remove the panel on logout. The opaque key scopes UI retention only; it
   is not submitted as authority and cannot prove which host produced a report.
3. Keep cache data bound to the accepted lifetime and current grants on the
   server. The proposed wire carries no host/boot identity, so a frontend key
   cannot detect a wrong-host cached response. GET must project original samples
   at current server time, withhold wrong-owner/revoked data and avoid collection
   or writes. Focus/online and explicit refresh read only that cache.
4. The existing qualification owner accepts the mounted authenticated panel:
   principal/grant/host/boot changes, logout, focus/reconnect, late reads, partial
   expiry, permission/error states and real service observations. Local key,
   controller and static markup tests do not prove React/browser lifecycle or
   installed middleware. Browser visual/responsive review remains required.
   Installation, configuration writes and restart/recovery stay gated by the
   existing central authority, package, access-policy and storage requirements.

## Local checks and limits

Pinned TypeScript checking passes. **161 frontend cases** pass across nine files,
including 38 new tools cases. Cases exercise the shared Rust wire fixture,
independent age 90/91 boundaries, privacy, invalid clocks, coalescing, cancellation,
stalled bodies/deadlines, read/access failures, original deadline rearming,
unavailable ownership, owner keys, unique heading relationships and late abandoned
results. Existing frontend cases run once per file, sequentially, retaining their
actual case counts. No new Rust test result is credited; those sources/fixtures
are unchanged from the prior tools candidate.

The standalone synthetic preview builds with pinned Vite/React and existing
CSS. It includes current, partially expired and unavailable examples; its bundle
is not the mounted dashboard or a performance measurement. The browser tool
returned `{"apps":[],"browsers":[]}`, so visual review could not run. No preview
server was started, no screenshot was produced and no browser acceptance is
claimed. The initial test-loader failure and final passing checks are preserved:
omitting the source filename made TypeScript's test transpiler parse a `.ts`
generic arrow as JSX. Giving the helper each real filename fixes that test seam.

Production route/cache/contracts/shell registration, real host lifetime, installed
service/application/recovery, ARM64/Pi, performance/soak, defect closure and phase
or cutover acceptance remain open. No production host was contacted.
