# B2 catalog-backed stack discovery

Engineer B, 15 October 2026. Independent branch from main `b37c718`.

The current dashboard groups Engine labels and loses declarations with no
containers. Deliver a bounded read-only inventory, genuine-reader projection,
authenticated cached-read handler and stack/member UI. Shared Store, worker,
protocol, route and shell registration remain integrator-owned. This slice does
not finish B2, enable lifecycle actions or qualify an installed host.

Use the existing protected management catalog and one complete Engine membership
snapshot. A label-only view cannot discover empty stacks. A new disk scanner or
per-stack Compose commands would duplicate protected collection and polling.
Instead, project the existing genuine owners into one pure domain reconciliation;
the central observation worker validates/recollects and caches its result.

Declared stacks remain visible with zero members. Services list all replicas and
one-offs separately. A running one-off cannot satisfy a missing service. Running
means every declared service has ordinary containers and every ordinary container
is running; undeclared services remain visible as registration drift. Empty means
no ordinary containers observed, not safe to delete or absence of storage users.
Non-running is reported as stopped without inventing Engine health/exit state.

Join only exact engine, project, case-sensitive working directory and ordered
configuration-file identities. A reused project name, changed file order or
overlapping foreign project produces a conflict, with no inferred safe counts.
Engine-only projects stay separate; unavailable/stale catalog data cannot classify
them as definitively unregistered. Standalone containers are counted separately.
No source path is canonicalized, case folded or adopted as managed configuration.

File diagnostics are explicit read-only facts: not inspected, conflict,
unsupported source or unavailable. Matching catalog/Engine labels never proves
actual YAML/.env/include/interpolation equivalence. The genuine projection marks
files not inspected until the central file collector supplies diagnostics. All
actions remain unavailable, including when counts and labels match. Existing
protected source/review libraries remain unchanged and must be used for actions.

Freshness is explicit for catalog and Engine. Stale/future/incomplete Engine facts
produce unknown runtime and null counts, never an empty/stopped success. Fresh
Engine facts from a different engine cannot join catalog declarations. Catalog
failure keeps Engine projects visible with unknown registration. Preserve sample
times separately; building or reading a report never refreshes evidence age.

Bound registrations to 8, declared services to 16 each, Engine members to 16,
configuration files to 4, paths to 512 bytes, names to 64 bytes, aggregate input
text to 48 KiB and serialized output to 64 KiB. These match or conservatively
compose existing collection ceilings; they do not increase shared worker limits.
Validate before cloning and sort output deterministically. Out-of-budget input
fails explicitly rather than dropping members or truncating the inventory.

Prepare a GET-only authenticated handler using a cached backend contract. GET
must not scan files, launch Compose, trigger Engine inspections or persist state.
The UI uses the current dashboard's restrained tables, badges and expandable
service detail. Show exact stack/project/path, source failures, sample freshness,
missing service containers, one-offs and registration drift. Refresh/reconnect
reads coalesce, abort on identity changes and discard late responses. Expired or
forbidden sessions clear private data. No stack actions appear in this slice.

Focused tests cover empty/stopped/replica/one-off layouts, unrelated projects,
identity/overlap conflicts, stale/unavailable/mismatched sources, filename/source
diagnostics, limits/order and no count fabrication. Use exact Rust leaf harnesses
and frontend tests; broad qualification remains with the existing test owner.
The frozen reference is pi-health `80593b29443ea9b81eb60ed4e5c3c3ac7a236204`,
especially `stack_read_service.py` and its tests; execute none of its Python.
