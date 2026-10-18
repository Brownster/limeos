# SnapRAID scrub planning

Stream A A3 continuation, based on main
`7df3f7530865c31b7faefd1f7b12a25aa7655860`. The existing pure protection planner
supplies configuration and sync eligibility, but no scrub preview. Implement
the missing typed scrub proposal within A-owned protection source/tests. Shared
roots, contracts, policy, worker/protocol, Store, effect registration and package
activation remain with the integrator. This is an A3 input, not installed parity.

## Frozen behavior

Reference `pi-health` at `80593b29443ea9b81eb60ed4e5c3c3ac7a236204`:
`storage_plugins/snapraid_plugin.py` defines manual numeric percent 0–100 and
minimum age 0, with UI defaults 8/10. Its privileged helper forwards only
explicitly supplied percent/age flags; absent flags use the selected binary's
defaults. Its unprivileged fallback instead supplies percent 12 and ignores
age. Saved configuration defaults are 12/10, and its schema requires configured
percent 1–100. Preserve the distinction between configured and manual values;
do not silently choose one of these different defaults.

The [official manual](https://www.snapraid.it/manual) confirms numeric 0–100,
age selection and scrub's content-file writes. Scrub is an effect, despite the
frozen UI's nondangerous label. No process or filesystem operation belongs in
this planner. Bad/new/full modes are not exposed by the frozen manual command
schema and remain outside this numeric adapter, rather than accepting free text.

## API and safety

Add `protection::scrub` with `SnapraidScrubRequest`, `SnapraidScrubPreview` and
`plan_snapraid_scrub`. The request has optional numeric percent and age_days,
preserving explicit zero and omission. Percent above 100 is invalid. Age uses
the existing typed configuration's u16 bound; larger values are explicit
unsupported input, never clamped. Neither omission nor an absent optional
field inherits saved settings. A future accepted runtime must retain that
intent and bind its fixed package/binary defaults in the authorized plan.

Reuse `SnapraidConfig::plan` and the existing complete fresh source validation:
all source/data/content/parity paths, physical UUID/serial/filesystem identity,
unique writable host mounts, original five-second evidence, boot/swap exclusion,
pool-root/generated-leaf exclusion and no hidden nested mounts. Scrub additionally
requires existing regular content/parity files. Sync may still admit safely
absent files for initial creation. No new sync restriction is introduced.

The preview retains the exact rendered configuration, original evidence and
request. It is serialized planning data with no provenance, approval, locks,
receipt or executable authority. Overall disabled protection refuses as sync
does. A manual request does not depend on timer flags or saved scrub-enabled
settings; recurring authorization and scheduling belong to B/central. Scrub
uses no sync diff thresholds or force bypass and cannot be represented as a
successful health observation before real execution and verification.

## Alternatives and checks

Reusing sync planning would attach unrelated diff/threshold behavior and allow
initially absent inputs. Creating an independent source checker risks drifting
from SRA-001/002. Reuse the current checker, then apply the command-specific
existing-file requirement in this isolated child module.

Use frozen layout fixtures and existing protection evidence builders. Test
manual option boundaries, configured/manual default separation, missing or
unsafe every-role sources, stale/replaced identity/namespace/hidden mounts,
disabled config and pool exclusions. Pair an absent-file scrub refusal with
existing sync acceptance. Run focused pre-change protection cases, final domain
tests and strict Clippy sequentially in the shared disk cache, one Cargo job.
No SnapRAID binary, Pi or installed qualification suite is run. Handoff exact
source and input hashes plus central runtime obligations; A3/P04 and existing
resource/performance/installed defect gates remain open.
