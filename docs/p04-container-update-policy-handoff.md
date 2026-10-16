# Container-update policy integration handoff

Engineer B, 15 October 2026. Branch `engineer/p04-container-update-planning`,
based on B2 `5e275ca`. Tested code: `46f63015d380f20ca31768d188c02e0f4e60e786`.
This branch includes B2, not B4 or the package/HOST-001 branch. It delivers pure
selection and diagnostic convergence comparison; RW-044 execution stays open.

October 15 follow-up: [image-content admission](p04-image-content-handoff.md)
supersedes the draft digest-source binding with version 2, separating an immutable
root (index or manifest), selected platform manifest and runtime config digest.
The earlier evidence below retains its original version-1 source identity.

## Delivered behavior

`crates/domain/src/deployment/container_updates.rs::plan_updates(UpdateInput)`
accepts plain supported declarations, exact selected stack aliases, B2 catalog/
Engine facts and platform-specific image resolutions. It returns an opaque
bounded `UpdatePlan` with no deserializer, effect method or authority shortcut.

- Preserve frozen update defaults, exclusion list, notification preference and
  exact disabled/daily-04:00/weekly-Sunday-04:00 preset names. Scheduled selection
  needs enabled plus a non-disabled preset. Manual selection can run with
  scheduling disabled, as the reference allows. Unreadable config stays unknown.
  This config DTO covers settings only. Legacy `last_run`/`last_run_result` still
  need an explicit adapter/migration to central job history; extra fields are
  refused rather than silently discarded.
- The selected stack set is explicit. Exclusions yield no service targets and
  never expand the request. The public summary includes only selected before
  stacks; unrelated inventory remains outside the review response. Current
  permissions must still cover every exposed selected stack/resource.
- Reuse B2's exact engine/project/case-sensitive directory/ordered-file join.
  Require a complete supported service/source/platform/replica specification.
  Source conflicts/unsupported diagnostics, registration/service/replica drift,
  stopped/empty stacks and active one-offs block proposals. Stopped one-offs
  remain visible but never satisfy ordinary replica presence.
- Compare every ordinary replica's image ID with the resolved image **config**
  digest. Preserve the manifest digest separately. Layer-download strings,
  successful pull/Compose exit and empty output are not change/convergence proof.
  No command-output input exists. A changed config can propose recreation even
  without download text; a changed manifest alone need not recreate containers.
- Resolution engine/project/source/platform must match the selected declaration.
  A pinned source cannot move to a different manifest. Missing/stale/future
  resolutions stay unknown. Original source/resolution timestamps are retained;
  a new plan/read time does not renew their age.
- Sort rows and service targets; retain the source, platform, expected replicas,
  immutable manifest/config identities and recreation comparison in the draft.
  `Current` describes observed image-ID alignment only, not full Compose/source
  equivalence, health, authorization, a written pin or successful execution.

`assess_post_update` compares later plain observations with that retained draft.
For proposed updates, require fresh matching catalog/Engine identity after the
dispatch boundary and original samples, exact selected service/replica scope,
every ordinary replica running on the expected config, and unchanged stopped
one-offs. Missing/stale/future/pre-dispatch reads stay unknown. Mixed images,
stopped/missing replicas, changed scope/engine and one-off changes cannot converge.
Current/excluded/disabled/blocked drafts yield no update outcome. The result is
`observed_converged`, never a protected receipt or a service-health claim.

Limits: 8 selected/spec stacks, 16 selected services total, 16 image resolutions,
8 exclusions, 4 ordered files, 64-byte aliases, 512-byte paths, 192-byte repository,
128-byte tag, canonical SHA-256 identities, 32 KiB additional input text and 64 KiB
complete review summary. B2's existing 48 KiB input/64 KiB output and 16-member
ceiling stay unchanged. Oversized selected counts and aggregate resolution text
fail admission before new draft clones. These DTOs are not bounded IPC decoders.

## Integration requirements

1. Add `pub mod container_updates;` beside existing domain deployment exports.
   B4 and package work also touch `deployment/mod.rs`; combine their exports.
   The isolated harness registers this dormant leaf only for focused compilation.
   Shared manifests, production lockfile, root exports, protocol and Store are
   unchanged; existing serde dependencies suffice.
2. Supply complete supported declarations from genuine protected Compose source
   owners, including exact current source identity/content. `FileStatus::NotInspected`
   allows a conditional draft only; source admission remains an unresolved gate.
   No YAML/.env/include/profile/build/VPN/network or full-template adapter is added.
   Never discard unsupported fields to fit `StackUpdateSpec`.
3. Collect and authenticate image resolution through the common protected worker.
   Resolve the supported platform manifest/config chain; distinguish OCI index,
   manifest and config identities; verify content/origin and local availability.
   DTO field names/digests are not proof of that chain. Preserve original owners,
   lifetimes and whole-worker time/FD/memory/report budgets. There is no registry
   client, Engine image reader or pull implementation in this leaf.
4. Bind the exact target source/manifest/config/platform, complete actual files,
   all containers (including one-offs), networks and physical dependencies into
   current authority, review, common claims/locks and the durable operation path.
   A supplies trustworthy storage dependencies; shared integration owns that seam.
   Recollect/revalidate before each effect. Never dispatch from plain input DTOs
   or treat an opaque proposal as executable approval.
5. Generate/apply image changes through the existing reviewed managed-override
   and lifecycle path, preserving other fields, operator bytes and numeric/path/
   network contracts. This planner generates no Compose artifact. A whole-stack
   apply must not implicitly start stopped services or widen dependencies. Do not
   substitute unrestricted `compose up` for the supported reviewed operation.
6. Scheduled mode here evaluates configuration only; it does not compute due
   times, maintenance windows, retries or cooldowns. Keep B1's durable occurrence/
   job identity and current independently reviewed recurring policy. A one-time
   approval does not cover later runs. New stacks/services/source/platform/target
   changes must not inherit broader authority merely because discovery found them.
   The frozen presets are local wall-clock cron metadata: timezone/DST migration
   still needs an explicit contract; do not convert them to UTC intervals silently.
7. Execute registered `202` jobs with atomic managed-file before-images/recovery,
   verification and retained barriers for uncertain effects. A process exit code
   and this diagnostic post-comparison cannot create a success receipt. Verify
   actual source, expected runtime configuration, service health and protected
   identity independently. Do not retry an uncertain recreation blindly.
8. Recheck current read/mutation scopes through central API/UI registration.
   Publish success/update notifications only after accepted verification commits,
   using the ordinary notifier and preserved preference. Show blocked/unknown/
   failed/ambiguous outcomes. Reconnecting must read the existing job, not relaunch
   it. No route, UI mutation, notification delivery or recurring policy is installed.

## Evidence and acceptance

[Local evidence](rewrite-evidence/p04/2026-10-15-container-update-policy/README.md)
records 24 focused Rust tests, strict Clippy, Rustfmt and harness lint/format.
Frozen config defaults and four image/text regressions are fixtures. Tests cover
replicas/one-offs, stopped stacks, source/platform/scope/engine drift, stale facts,
pre/post boundaries, incomplete convergence, strict fields, requested-only summaries,
selected-service counts and aggregate resolution bounds. The 12 inherited B2
tests are compiled but filtered and retain their earlier evidence identity.

Qualification owner still needs genuine supported source/image collection,
digest-chain/platform/origin refusal, scoped API/UI/reconnect, fresh and recurring
authority, maintenance/timezone behavior, locked storage dependencies, exact image
apply, health checks, notification failures and interruption/lost-response recovery
on the registered slice. No Docker/registry/Pi command or host effect, broad suite,
ARM64, package, performance or cutover qualification occurred here. No defect or
RW-044/P04 milestone is closed by these library tests. A's active files stay intact.

References: frozen pi-health `80593b29443ea9b81eb60ed4e5c3c3ac7a236204`,
`update_service.py`, `update_scheduler.py`, `tests/test_update_service.py`;
[Compose pull](https://docs.docker.com/reference/cli/docker/compose/pull/),
[image inspection](https://docs.docker.com/reference/cli/docker/image/inspect/),
[OCI manifest](https://github.com/opencontainers/image-spec/blob/main/manifest.md),
[OCI config](https://github.com/opencontainers/image-spec/blob/main/config.md).
