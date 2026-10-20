# A1 installed namespace progress, 20 October 2026

The initial installed profile cannot admit an ordinary Debian host: baseline
`477b81f` reaches a ninth mount namespace against its limit of eight, with no
Engine containers. The [source-bound packet](rewrite-evidence/p04/2026-10-20-a1-namespace-budget/README.md)
preserves the exact-package reproduction, correction, native checks and failed
follow-up attempts. This work does not complete A1 or authorize Pi cutover.

## Source changes

- `509c05a`: select the 13-FD `InstalledMountV1` configuration profile for the
  four fixed installed paths. Sixteen complete namespace anchors now fit a
  213-FD aggregate reservation within the existing 226 model. Service limits,
  protected reads, leases, completeness and recovery barriers remain intact.
- `0934eb4`: distinguish kernel/userspace/mixed task changes from namespace
  membership and mount-table changes in bounded root diagnostics. Every earlier
  comparison and poisoned-owner rule still applies.
- `0c97fcb`: reuse the foundation denial probe after an early peer close. Test
  the same permitted inspection with Core and nobody, with successful authorized
  reads before and after denial. Unsupported Health could not prove this UID
  boundary because the closed target role forbids it for both UIDs.
- `d579f81`: retain bounded stdout, stderr and exit code on a failed fixture
  command. `187b248` declares the new shared fixture dependency in the isolated
  schema-upgrade reference test.

The final native checks pass: 216 storage unit cases, 44 executor-library cases,
13 storage compile-fail doctests and 61 reference cases. Affected-crate all-target
Clippy and Rust formatting pass. Repeated focused runs are not added to these
counts. The full workspace, frontend and native ARM64 CI results for `477b81f`
retain their original source identity; they do not qualify this newer source.

## Actual installed result

Two Debian-compatible executors were built offline in the pinned existing local
Rust 1.88 image. The second checks all 656 frozen inputs inside the builder before
compilation. Derived fixture packages retain the unchanged qualified CI Core,
CLI, password worker, UI, contracts and units. Their rebuilt executor and profile
documentation, exact binary/package hashes and all private wrappers are recorded.

The new executor passes the old namespace overflow, then refuses a genuine host
revalidation: `scan.recheck.user_tasks`, 116 tasks against 118 previously retained,
HTTP 409 before plan creation. The disappearing identities are not established.
No identity, namespace or mount-table comparison was dropped or renewed.

The subsequent witness path exposed the corrected early-close test issue. Later
full-suite attempts were stopped by the unchanged sampled 3-GiB host memory
threshold, including a 768-MiB guest with the same tests/service ceilings. That
guest passed the strengthened UID probe and predecessor installed recovery and
package tests but did not finish A1. An earlier tune2fs fixture failure did not
recur after improving its diagnostics; its original cause is still unproven.
Interrupted or failed runs supply no accepted-mount result.

## Next integration work

Finish the synchronous before/after process witness when host headroom permits.
The complete input is retained under `.cache/a1-namespace-review/`, including
`diagnostic-candidate/`, `proc-witness-v3-guest/`, `run_768.py` and their manifests.
The runner changes only non-build guest memory; it preserves every assertion and
service limit. Identify the actual task change before changing runtime policy.
Then qualify genuine approval, mount, independent verification, replay and restart.
Broader consumer/crash/resource/performance/ARM64 gates remain separate.

Continue B2's [registered stack inventory](plans/2026-10-20-stack-inventory-runtime-design.md)
while heavy VM qualification lacks headroom. Its unfinished initial draft remains
in `.cache/main-consolidation/recovery/stack-inventory-start/`, not accepted source.
The target remains genuine supervised collection, shared cache, current stored
authorization, registered API and mounted UI, followed by shared stack effects.
The remaining A/B runtime, P05 assistant and P06 migration/return requirements are
unchanged. No production Pi access, frozen-app execution or push occurred here.
