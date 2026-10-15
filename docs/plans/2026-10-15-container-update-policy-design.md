# B container-update selection and convergence policy

Engineer B, 15 October 2026. Follow B2 `5e275ca`. Shared Store/authority/dispatch
contracts remain pending; A's mount integration has active changes. This slice
adds no registry, locks, receipt, SQL, worker launch, route or host effects.

Preserve frozen update configuration defaults, exclusions, notification preference
and disabled/daily-04:00/weekly-Sunday-04:00 preset names. Scheduled evaluation
requires enabled plus a non-disabled preset; explicit manual evaluation preserves
the frozen ability to run while scheduling is disabled. Each request names a
bounded exact stack set. Config unreadable is unknown, not a defaulted permission.
Preset metadata does not translate local wall-clock cron to UTC intervals; timezone
and DST migration still require an explicit central contract/P06 explanation.

Use B2's exact engine/project/directory/ordered-file membership join. Compare each
ordinary replica's Engine image ID to a resolved platform-specific image config
digest. Require a complete supported service/source/platform/replica declaration
and exact matching resolution for every service. Unknown/stale/future images,
identity/source/platform drift, service/replica drift, stopped/empty stacks and
active one-offs block or remain unknown. Do not auto-start stopped services.
Stopped one-offs remain visible and cannot satisfy ordinary replica presence.
Explicit source conflicts/unsupported diagnostics block proposals.

Keep tagged repository/source, immutable manifest digest and image config digest
separate. A pinned source cannot move to a different manifest. An OCI index or
manifest digest is not the container's image ID. The frozen download-text heuristic
is deliberately replaced: layer progress, successful pull/Compose exit and empty
logs are neither image-change nor convergence proof. Add no pull-text input at all.
The plain resolution DTO is not a trusted resolver or proof of content integrity.

Return an opaque bounded review draft with sorted per-stack/service changes,
original source timestamps, full before inventory and unresolved central gates.
It generates no Compose artifact. The integrator binds accepted targets into the
existing managed-file/lifecycle machinery, preserving all other configuration.
Exclusions do not bypass input validation or expand the requested resource set.
Changed candidate/registration/resource scope needs current review/recurring policy.

Add pure post-update observation assessment against the retained draft. Require
post-dispatch fresh matching catalog/Engine identity, exact declared services and
ordinary replica counts, all ordinary members running on the expected config IDs,
and unchanged stopped one-offs. Stale/missing observations remain unknown; mixed
images/stopped replicas or scope drift do not converge. This produces a diagnostic
comparison only, never a protected success receipt, health claim or retry decision.

Limits: 8 selected/spec stacks, 16 selected services total, 16 image resolutions,
8 exclusions, 4 files per identity, 64-byte aliases, 512-byte paths, 192-byte
repository, 128-byte tag, canonical SHA-256 identities, 32 KiB additional input
text and 64 KiB complete review report. B2's existing 48 KiB input/64 KiB output
and 16-member ceiling remain unchanged. Admission precedes new clones; excess
data is refused, never silently truncated. Plain Deserialize is not IPC admission.

Focused fixtures/tests cover frozen config behavior, exclusions, config-ID changes
without log hints, layer downloads without changes, replicas/one-offs, engine/
source/platform/identity mismatches, freshness, strict fields, scope and bounds,
and post-update convergence/refusal. No Docker/registry/production/Pi commands,
installed or broad qualification run. B3 effect/recovery and B1 durable recurring
authorization are still required before any scheduled update can execute.

References: frozen pi-health `80593b29443ea9b81eb60ed4e5c3c3ac7a236204`,
`update_service.py`, `update_scheduler.py`, `tests/test_update_service.py`;
[Compose pull](https://docs.docker.com/reference/cli/docker/compose/pull/),
[image inspection](https://docs.docker.com/reference/cli/docker/image/inspect/),
[OCI manifest](https://github.com/opencontainers/image-spec/blob/main/manifest.md),
[OCI config](https://github.com/opencontainers/image-spec/blob/main/config.md).
