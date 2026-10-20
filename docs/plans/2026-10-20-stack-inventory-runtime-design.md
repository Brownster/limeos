# Registered stack inventory runtime

Start from main `477b81f`. The current dashboard shows Engine-label stack totals,
while B2's catalog/member view and genuine retained reader remain unregistered.
Connect that existing read model through one supervised cohort, a shared cache,
current session/grant checks and the authenticated Stacks view. This is the next
B2 integration milestone toward the operator's metrics/container/stack minimum;
it does not replace the remaining A/B, P05 or P06 requirements.

Estimate: 32 human engineering hours; scope review at 48. Actual human effort is
unknown. P04 retains its existing 320/480-hour planning bounds.

## Trusted collection

Reuse the genuine UID-0 management-catalog and fixed-root Engine Compose owners.
The ordinary container service is non-root and cannot substitute for those
constructors. Use the existing dormant root storage-reader profile, whose host
mount view and 64 MiB/256-FD/16-task limits already support protected reads.
Add a distinct read ceiling and closed request; container mutations remain
outside this role. Never make Core or an HTTP handler perform catalog/Engine I/O.

A dedicated fixed package-owned worker image runs through the existing sealed
image, guardian, independent four-second supervision, cancellation and verified
cleanup machinery. Extend that machinery with a closed stack selection rather
than inventing a new process tree, executable argument or caller-selected command.
The configured catalog name is protected configuration, not browser input.
Share admission with other root-reader work, so a cohort cannot overlap another
expensive collection in the same service. Preserve poisoned cleanup barriers.

Reserve all catalog/Engine/report capacity before collection. Keep the existing
8-stack, 16-member, 16-service, four-file, 48-KiB selected-text and 64-KiB report
bounds. Do not increase existing service limits. Use a narrower stack-worker
payload/RSS profile within the existing whole-service allowance; account for
manager, guardian, sealed image, worker and transient descriptors. Component
models and sampled guards are not measured whole-service acceptance.

Keep original owner leases and clocks across final catalog/Engine revalidation.
Project selected display facts only after the genuine owners pass. No serialized
catalog, marker, JSON report or cached value reconstructs retained authority.
Catalog and Engine failures remain separately visible. Missing catalog data is
unknown registration, never a proven empty registry. All lookup and worker
failure paths preserve uncertainty instead of generating successful empty facts.

## Shared cache and HTTP

One background collector serves all readers. RPC and HTTP GETs return cached
facts, never launch probes. Preserve sample timestamps, monotonic age and the
original source identity. Repeated GETs, reconnection and background retries
cannot refresh failed or expired observations. Retained prior facts are visibly
stale/unavailable; counts and definitive registration relations are withheld
when their required source fails. Clock rollback cannot revive a cached cohort.

Use bounded display-only wire decoding and generated contracts. Validate complete
cardinality, fields, ordering, identities and response size before caching or
serializing. A report is a past observation, not a physical/namespace proof or
execution approval. Retain the protected descriptor/source/effect boundary for
subsequent whole-stack mutations and A's dependency-aware shutdown integration.

Register `/api/v1/deployment/stacks` in the existing router and middleware.
Authenticate the current session and stored grants before reading private cache
data. Require permission for every disclosed registered stack/project/container;
partial grants cannot leak member names, paths, image identities or counts.
Fail the complete response when a safe complete view cannot be authorized.
No role label or serialized permission flag supplies authority.

## Native UI and qualification

Mount the existing stack/member view in the authenticated Stacks screen, using
generated types and principal/grant-bound ownership. Dispose abandoned requests
at the browser commit boundary. Keep current container jobs and explicitly
approved per-container controls available separately. Empty declared stacks,
stopped members, replicas, one-offs, extra services, identity conflicts, source
diagnostics and age remain visible. Whole-stack actions require the later shared
claims/approval/receipt/recovery integration; this report cannot enable them.

Before accepting the slice, verify real collection/refusal and resource admission,
worker death/deadline/late-output/cleanup, bounded RPC decoding, failed/stale
cache sources, current authorization and no-probe GET behavior. Cover the
registered Core/API path and mounted UI lifecycle in addition to existing pure
fixtures. Preserve earlier failed attempts and source-bound evidence. Coordinate
installed/root/ARM64 qualification on an immutable snapshot; do not inherit the
current `477b81f` CI result for newer source.

The live baseline CI run is `39429582703`: its native ARM64 job passed, while the
last inspected AMD64 job was still building Debian packages. This is a specific
live qualification handle, not evidence for this new implementation.

Keep work, compilation, caches and ordinary scratch on repository disk, one local
Cargo job at a time. Use the current checkout on an isolated branch to reuse its
large cache rather than duplicate artifacts with only 20 GiB free. The eight old
dirty worktrees remain untouched. No new production Pi window, host mutation,
package activation, phase closure or cutover is authorized by this design.
