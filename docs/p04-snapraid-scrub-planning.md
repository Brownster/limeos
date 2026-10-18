# SnapRAID numeric scrub planning handoff

Stream A A3 input on `engineer/p04-snapraid-scrub-planning`, based on main
`7df3f7530865c31b7faefd1f7b12a25aa7655860`. Tested product source:
`808a6fe9118ee1b959daf14c87b7ceaf34fb941a`.
See the [design](plans/2026-10-18-snapraid-scrub-planning-design.md) and
[local evidence](rewrite-evidence/p04/2026-10-18-snapraid-scrub-planning/README.md).
This adds the missing pure scrub proposal beside the existing sync planner.
It does not register a scrub operation, process, route, schedule or UI.

## Supplied API

The existing public protection module now exposes
`limeos_domain::protection::scrub::{SnapraidScrubRequest,
SnapraidScrubPreview, plan_snapraid_scrub}`. No crate-root re-export or shared
generated contract was changed.

`SnapraidScrubRequest` contains optional numeric `percent` and `age_days`.
Omission and explicit zero stay distinct. Manual percent accepts 0–100;
configured `SnapraidConfig.scrub.percent` retains its existing 1–100 rule.
Age uses the existing configuration's bounded u16 representation, 0–65,535.
Overflow, fractions, numeric strings and free-form options are refused, never
clamped or coerced. Unsupported larger ages remain visible rejected input,
not an implicit migration retirement. `bad`/`new`/`full`, force flags, arbitrary
configuration/log paths and command text are not admitted by this numeric
request. These modes are absent from the frozen manual command's parameter
schema; this is not a claim that SnapRAID itself lacks them.

The request does not inherit saved scrub configuration. Frozen `pi-health`
`80593b29443ea9b81eb60ed4e5c3c3ac7a236204` has different defaults: manual form
8/10, saved settings 12/10, helper omission using the installed binary, and
unprivileged fallback forcing percent 12 while dropping age. The preview
preserves the privileged path's explicit parameter intent. It neither silently
substitutes settings nor adopts the fallback's dropped-age behavior. Before
execution, central registration must bind the selected package/binary and its
effective defaults in the authorized plan and display them accurately.

`plan_snapraid_scrub(config, pools, contract, sources, request, now)` reuses the
existing configuration and complete source checks. It retains the exact
rendered configuration, original source evidence/time and request in the
preview. Overall disabled protection refuses. Manual requests do not enable
timers or depend on saved scrub/schedule flags. The function applies no sync
diff thresholds and provides no override. A paired regression proves a diff
that blocks sync is not silently treated as scrub's prerequisite or approval.

All source/data/content/parity requirements must pass the same original
five-second evidence window, exact path/device/mount bindings, UUID uniqueness,
expected serial/filesystem, unique writable host-root filesystem mounts,
boot/swap exclusion and no hidden nested/bind/pool mounts. Additional scrub
admission requires existing regular content/parity files. Sync retains safely
absent file admission for initial creation. No missing scrub input is created,
repaired or converted into healthy status by this planner.

## Runtime handoff

The [official manual](https://www.snapraid.it/manual) documents scrub's content
metadata writes. Treat it as an effect under the shared locks and recovery
model. Do not extend the read-only `Status`/`Diff` query enum to execute scrub.

The preview and serialized source summary are ordinary review data. A future
registered operation must load the protected configuration selection, collect
genuine original source owners with complete physical/shared claims, check
current authority and obtain fresh approved execution under those locks.
Recollect/recheck at the effect boundary; cloning/serializing a preview cannot
refresh evidence lifetime or create authority. Bind requested/effective numeric
options, exact package/configuration and source identities into that operation.
No caller-supplied command, path or reusable recurring approval is accepted here.

Use a fixed supervised executor adapter with real termination, bounded output,
deadline, cancellation/cleanup and verified outcome. Keep content-file recovery
and durable receipts in the shared journal; dead or unverifiable work remains
uncertain. A successful preview says nothing about actual array integrity,
content validity or completion. An unsynced array's possible scrub diagnostics
must remain visible; no synthetic clean diff or healthy result is produced.

B/central still own recurring policy, current-grant checks at each dispatch,
overlap exclusion and notification/status registration. A manual request and
saved enable flags supply no recurring authority. The qualification owner
still needs installed all-role refusal, actual scrub result/content behavior,
interruption/cleanup/restart and native API/UI acceptance. Complete A3 also
retains MergerFS, sync/configuration, Samba and the associated dependency/runtime
requirements. This leaf closes none of A3, SRA-001/002, P04 or cutover.

## Local checks

All 157 domain unit tests and three existing compile-fail doctests pass,
including 14 new scrub tests. The unchanged focused protection baseline passed
11 cases with 132 filtered before implementation. Strict all-target Clippy,
Rustfmt and whitespace checks pass. Tests use existing frozen layouts and
synthetic source facts; they execute no SnapRAID binary or host operation.

Two initial runs each had one fixture failure: the reorder and duplication
mutations used identical source/data directory observations. Correcting both
to distinct paths made the intended negative cases real; production planner
logic was unchanged. The final fixture asserts every mutation changes facts.
Both failed logs are retained. No installed, resource/performance or hardware
qualification is claimed. The historical inventory p95 61.401 ms / 20 ms remains
failed; A4/A5 and P05/P06 remain separate unfinished work.
