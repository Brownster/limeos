# B4 catalog planning integration handoff

Engineer B, 15 October 2026. Branch `engineer/p04-catalog-planning`, based on
B2 `5e275ca`. Tested catalog code: `39039fd73496b25c9098659b6990953f94588320`.
The branch includes B2; do not treat its history as an independent main-based
delivery. This adds pure domain proposals, not registered install/remove jobs.

## Delivered behavior

- Explicit existing/new stack selection and `(app, stack)` removal prevent
  first-match targeting when an app exists in multiple stacks. New-stack planning
  checks supplied names only; it establishes no actual directory absence.
- Operator and managed declarations stay separate. Catalog services must have
  distinct managed names. Operator service replacement/removal is refused.
  Only a complete replacement `limeos.override.json` artifact is generated.
- Typed top-level networks, volumes, configs and secrets survive installation.
  Equal definitions reuse existing resources; different definitions conflict in
  each section. Operator-only resources remain inherited from operator YAML.
  Existing managed resources, including unused definitions, remain in the file.
- Config/secret service grants and explicit Compose dependencies survive rendering.
  Catalog `requires` remains business metadata, separate from `depends_on`.
  Missing references, dependency cycles, duplicate list names and conflicting
  grant/mount/device targets are refused. Dependents block removal.
- Removal changes just the selected managed service. Other services and all
  resource definitions remain. The summary identifies the selected service that
  needs verified shutdown and retains explicit per-stack orphan policy.
- The existing compiler still checks pinned images, numeric UID/GID, literal
  mounts, ports and devices. Case-sensitive paths and supported resource names
  are retained. No file is read, rendered template substituted or process run.

The supported subset includes bridge/external networks, named/external volumes
and file-backed/external configs/secrets. Unknown fields fail deserialization.
Inline contents, environment sources, driver options, VPN namespace semantics,
grant UID/GID/mode and other unrepresented source fields need explicit adapters.
Adapters must refuse unsupported source, not discard fields to fit this model.
This is not qualification of all frozen catalog templates.

Bounds: 8 supplied stacks, 16 selected effective services including the incoming
service, 8 entries per service list, 16 resources per section per source, 48-byte
aliases, 512-byte resource paths, 128-byte external names, 64 KiB selected input
text/artifact and 256 KiB review summary. Count/text admission precedes project
clones; serialized output has a separate streaming bound. Unselected project
contents are neither cloned nor deeply inspected. Plain DTO Deserialize derives
are fixture/declaration conveniences, not bounded IPC or protected evidence.

## Integration requirements

1. Integrate B2's domain export first or take this branch as its follow-up. No
   root manifest, protocol, Store, schema, registry, API or UI mounting changes
   are supplied here. The standalone harness registers the dormant leaf only for
   local compilation; the production root still needs integrator registration.
2. Add complete supported template/source adapters using protected owners. Keep
   operator bytes/comments/anchors intact; verify exact ordered source and
   managed-file preimages, dependencies, relative-file base paths, numeric
   ownership and external resource identity. A declaration alone proves none of
   these. Resolve config/secret file ownership without loading secret contents
   into jobs, reports or debug logs.
3. Bind the complete before/desired catalog model, artifact and orphan policy
   into authenticated durable review. `base_service_impact` projects only the old
   `ComposeService` fields: it excludes resources, grants and dependency metadata.
   Existing P03 preview approval cannot authorize this larger proposal.
4. Use central executable authority, current grants, full stack/file/container/
   network/storage claims, common locks, atomic managed-file recovery and fresh
   physical/runtime/source facts. All eight existing Compose gates and seven
   catalog gates remain unresolved. There is no effect method, receipt flag,
   approval shortcut or alternative resource registry in this leaf.
5. For removal, obtain an immutable verified receipt for the selected service's
   stop before replacing configuration. A failed/uncertain stop aborts the edit;
   never down the whole stack as a shortcut. Reconcile orphan/container removal
   under reviewed policy. Empty resulting models need an explicit runtime path;
   do not assume `compose up` succeeds for an empty project. Retained data and
   resources must not be garbage-collected as a removal side effect.
6. Provide `202` durable jobs and reconnect-safe API/UI targeting/status through
   the existing operation path. Reads remain cache-only. B3 execution and B1's
   durable schedule registration still need the integrator's shared contracts.

## Evidence and remaining acceptance

[Local evidence](rewrite-evidence/p04/2026-10-15-catalog-planning/README.md)
records 20 focused Rust tests, strict Clippy, Rustfmt and Python harness checks
against the exact source commit. The frozen resource regression was adapted into
typed fixtures with a pinned image; it covers all four sections and grants, not
YAML parsing or real Compose deployment. Test-side map union checks inherited
operator resources. The 12 inherited B2 tests are compiled but filtered here;
their earlier evidence retains its own source identity.

| Defect | Local coverage | Acceptance still required |
| --- | --- | --- |
| CAT-001 | Exact service removal; other managed/operator services retained; dependent refusal | Two running services; only selected service stops; failed stop leaves files unchanged |
| CAT-003 | Four-section/grant fixture; equal reuse and collision refusal | Real supported template deployment with configs/secrets intact |
| STK-003 | Managed artifact only; inherited operator resources retained | Actual comments/anchors/source bytes preserved by the protected writer |
| STK-004 | Orphan policy present in summary | Reviewed orphan removal and recovery on apply |
| UI-006 | Same app in two stacks; exact target required | Registered UI selection, durable job and reconnect behavior |

No defect, RW-042 milestone or P04 phase is closed by this leaf. Installed parser
conformance, API/UI, verified shutdown, writes, crash recovery, concurrency,
ARM64/Pi and performance remain with central integration and the qualification
owner. No production host changed.

A's mount admission/effect/API/UI worktrees had active uncommitted changes at the
start of this slice. They were left untouched. Independent B4 work was available,
so taking over those A files would not have removed the shared execution blocker.

References: frozen pi-health `80593b29443ea9b81eb60ed4e5c3c3ac7a236204`,
`catalog_service.py` and `tests/test_catalog.py`; primary Compose
[configs](https://docs.docker.com/reference/compose-file/configs/),
[secrets](https://docs.docker.com/reference/compose-file/secrets/) and
[merge rules](https://docs.docker.com/reference/compose-file/merge/).
