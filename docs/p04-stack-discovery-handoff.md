# B2 stack discovery integration handoff

Engineer B, 15 October 2026. Branch `engineer/p04-stack-discovery`, independent
from main `b37c718`. Tested code: `c4b30f52e1d7f1cb318ff16e122a13f0cac219df`.
This is a read-only B2 slice. Runtime/API/UI registration, real source inspection,
installed qualification and the rest of B2 remain open. No lifecycle action is
enabled and no storage or host mutation occurs.

## Delivered behavior

- `crates/domain/src/deployment/discovery.rs` combines complete catalog/Engine
  facts. Declared stacks remain visible without containers. List each declared
  service, all replicas and one-offs, and additional services absent from the
  registration. One-offs never satisfy ordinary service presence/running state.
- Exact engine/project/case-sensitive directory/ordered file identity controls
  grouping. Changed identities and overlapping foreign projects withhold stack
  counts. Engine-only projects stay separate. Missing/stale/future Engine facts
  yield unknown runtime and null counts; stale/unavailable catalog facts cannot
  label projects definitively unregistered. Sample times remain unchanged.
- File diagnostics preserve ambiguity/unsupported source/unavailable facts. No
  diagnostic authorizes execution. The reader projection reports `not_inspected`;
  catalog/Engine agreement is not actual Compose source equivalence or safety.
- `crates/executor-container/src/deployment/discovery.rs::inventory` accepts
  genuine existing catalog and Engine owners, revalidates them using their
  original leases/ledgers and projects one inventory. This worker-side function
  performs child revalidation I/O; it starts no new collection and belongs nowhere
  in an HTTP handler. Returning a report creates no protected owner or permission.
- `crates/api/src/deployment.rs` proposes one authenticated GET,
  `/api/v1/deployment/stacks`, using `DeploymentBackend::stack_inventory(token)`.
  The backend must resolve current grants and read cached facts. Unsupported
  adapters return unavailable; GET cannot perform an inspection or effect.
- `frontend/src/StackManagement.tsx` supplies the read-only view and `StackPanel`.
  Member detail, exact paths, missing services, file conflicts, source ages and
  registration uncertainty are visible. The dedicated client bounds streamed
  JSON to 64 KiB and validates the versioned wire shape. The controller coalesces
  focus/online/manual refresh, expires retained counts without polling, aborts on
  identity cleanup and clears private data on 401/403. Late reads cannot restore
  an old session's view. Error messages from bounded server envelopes survive.

Domain bounds: 8 declarations, 16 services each, 16 Engine members, 4 ordered
configuration files, 64-byte names, 512-byte literal paths, 48 KiB aggregate input
text and 64 KiB serialized report. Input DTO Deserialize derives are convenience
for plain facts/fixtures, not bounded IPC admission or evidence constructors.
The genuine projection consumes the existing protected readers' smaller bounds;
none of the shared worker's ceilings or concurrency limits is increased.

## Central registration steps

1. Add `pub mod deployment;` to domain and executor-container roots. Existing
   dependencies suffice. Register approved wire contracts/generated types; the
   dedicated frontend types are the proposed version-1 shape, checked against
   actual Rust serialization by `wire.json`.
2. Extend the existing protected observation worker/protocol with a single shared
   catalog/membership cohort. Keep existing original owner lifetimes, whole-worker
   cancellation, read/FD/memory/time/report accounting and permanent-invalid-state
   rules. An unavailable/truncated collection cannot become `Some(empty)`.
   The public adapter has no independent launch, cache, lock or recovery model.
3. Cache bounded declaration/membership facts centrally. Reconcile freshness at
   read time using original timestamps; do not relabel retained facts fresh because
   a GET succeeded. `discover` accepts absent sources and zero declarations, but
   the existing protected management catalog requires 1–8 registrations. An
   absent/unreadable catalog is unavailable unless the central registry has an
   independently established complete-empty state; this leaf changes no schema.
4. Implement `DeploymentBackend` for Core with current session/grant checks and
   cache-only reads. Declare API `mod deployment;`, extend the `router` generic
   bound and merge `deployment::routes()` before existing shared middleware/state.
   Update the existing `Fake`, `Mutations` and `ReadBackend` test implementations
   with empty default trait impls when registering the new bound. Keep origin,
   security headers, concurrency and stream admission in the common API router.
   The harness's `proposed_routes` only compiles a leaf mount; it is not that shared
   middleware registration or an alternative production server.
5. Mount only in the existing authenticated Stacks view:

   ```tsx
   <StackPanel principal={session.principal.id}
     grantRevision={session.principal.grant_revision} expired={expired} />
   ```

   Remount/clean up on logout and principal/grant changes. This panel fetches only
   cached reads and offers no mutation controls. Preserve the existing container
   operations view separately; no action availability is inferred from this DTO.
6. Supply actual file diagnostics from protected collection. Current projection
   intentionally cannot claim YAML/.env/include/interpolation admission or hashes
   from declarations. Actions must consume the existing retained source/review
   owners, current authority, full claims, locks and recovery path. Zero Engine
   members never proves safe deletion/unmount or absence of physical consumers.

## Focused evidence and acceptance still needed

[Local packet](rewrite-evidence/p04/2026-10-15-stack-discovery/README.md): 16 Rust
tests (12 domain, 2 private reader projections, 2 handler tests), 10 runtime-layout
fixtures, 18 frontend tests, strict Clippy/TypeScript/Rustfmt and harness lint/format.
The wire fixture is compared to actual Rust output and decoded/rendered by the
frontend tests. Frontend checks render static React markup and exercise the
controller/client; they do not qualify a browser session or installed dashboard.
Private projection tests construct snapshots; they do not construct genuine
UID-0 Engine/catalog owners or qualify the public adapter on a host.

Qualification owner should accept the registered slice at a fixed source SHA:
empty/stopped/replica/one-off stacks; missing declared services and extra services;
wrong engine/case/file order and overlapping projects; collection refusal/partial
data preserving unknown counts; real filename ambiguity; read permissions and
logout/grant/reconnect behavior in the installed UI; no per-stack Compose commands
or per-dashboard Engine probes; whole-worker budgets and expiry/cancellation.
No RW-042, STK-005, P04 or cutover closure is claimed by these leaf tests.

## Stream B integration queue

| Slice | Branch head before this handoff | Central dependency |
| --- | --- | --- |
| B1 observation schedules/alerts/notifications | `engineer/p04-automation-b1` · `485206b` | Frozen Store/schema transactions, runtime/API/UI registration and installed deduplication acceptance |
| HOST-001 prerequisite planning | `engineer/p04-host-prerequisites` · `65ed86e` | Trusted collection, fixed operations, atomic recovery, package entry point and host verification |
| B2 stack discovery/member UI | `engineer/p04-stack-discovery` · tested code `c4b30f5` | Shared observation cohort/cache and authenticated route/shell registration; no new SQL migration required by this leaf |

These branches are independent from main and from each other. B2 can be integrated
without A's mount effects. B3 whole-stack execution still needs the central
executable-authority/claim/recovery contract and A's physical dependency evidence.
