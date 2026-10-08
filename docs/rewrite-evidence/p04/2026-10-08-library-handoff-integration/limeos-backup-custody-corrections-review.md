# RW-043 custody corrections independent review

Verdict: **GO for integration of the corrected custody library**, subject to full merged-source checks and publication of the correction evidence. All three findings in `/tmp/limeos-backup-custody-review.md` are resolved at the source below. No other correction is required by this review.

## Exact reviewed source

- Worktree: `/tmp/limeos-custody-boundaries`
- Original reviewed engineer tip: `483c963e669a1267c26e8f9390a8edff341ad2e7`
- Original measured implementation/tests: `9a8232262d8de552e10bd88126395cfe197ee634`
- Corrections: FIFO `3862f0a`, bounded serialization `2d4ee82`, guarded early cleanup `a869842`
- Clean corrected tip: `a869842a0273677f088d4fa563216ba091f1af29`
- `crates/backup-archive/src/custody.rs`: `a89218676b0fd9f88e1ebaa1b3b2dd74c96f632e86aff0f321cf52a149c7e77d`
- `crates/backup-archive/src/custody/tests.rs`: `d4fc2e3af33b96b8796d026e74744fef1ae9be137d91585a6242d7b165f3979a`

The corrective diff changes only those two source/test files. The engineer's original historical evidence and measurement source remain untouched; its 312 workspace tests and RSS measurements must retain their original attribution. No decoder, admission policy, manifest identity encoding, staging, dependency, route, runtime operation or live destination changes enter the corrections.

## Findings resolved

| Original finding | Settled correction and inspected proof |
|---|---|
| Recovery can block opening record/archive FIFOs before type validation | Private read opens add `O_NONBLOCK` while retaining no-follow and close-on-exec. The real FIFO subprocess regression preserves both FIFO replacements, refuses both without a writer, and kills/reaps a child on timeout. Pre-fix raw output records the record FIFO hang after 2.07 seconds; post-fix the same scenario passes in 0.22 seconds. |
| Canonical JSON is allocated before checking its byte cap | A counting writer stops at the trusted cap, including JSON escaping, before requesting the output buffer. Exact capacity reservation is fallible; a second bounded writer cannot grow beyond the count. Both publication and canonical recovery use this path. Tests prove exact-cap bytes equal the old canonical form, cap-minus-one refusal occurs before the private allocation seam, allocation failure remains typed, and omitted optional-field canonical amplification refuses before allocation. |
| Early failures unlink an unverified name, or file-vector allocation leaves an unguarded directory | Open/chmod/stat failure before ownership recording returns typed `CleanupFailure::Unverified` and retains the name. A known-inode `Pending` guard is armed before the fallible owned-file reservation. Real private replacements at open/chmod/directory-stat/archive-stat/record-stat retain their device/inode/content and the primary I/O failure. Allocation failure removes the known empty pending directory without touching the unrelated sentinel or selected source. |

The serializer uses borrowed `RecordRef` fields; `PolicySnapshot::policy()` returns `&AdmissionPolicy`, and the manifest is borrowed. Publication introduces no full policy or manifest clone before counting. The new cap governs canonical JSON output-buffer size and identity encoding, not every earlier admission allocation or recovery's typed serde heap representation. Admission remains governed by its existing policy; trusted input policy construction is the caller's responsibility. Recovery still checks bounded raw bytes before deserialization. Claims should say exactly that, without promising a total heap bound or a pre-admission small-record rejection.

The cleanup correction preserves the caller's existing exclusion-of-other-writers contract. Replacement/fault fixtures prove that cleanup does not unlink an unknown name; they do not claim an atomic stat-and-unlink primitive or protection against every concurrent same-UID/root writer. Unknown entries remain private, unadopted and reported. Known owned-name cleanup, fsync publication/recovery, ambiguous post-rename durability and the narrow inert legacy self-repeat behavior remain unchanged.

## Evidence inspected, without rerunning tests

Raw proof directory: `/tmp/limeos-custody-boundary-proof`. Independent data checks matched all nine raw output hashes in `sha256.json` and the exact settled source hashes. Inspected logs show:

- FIFO regression: one pre-fix failure, one post-fix pass.
- Canonical serialization/recovery regressions: two passes.
- Early cleanup replacement and allocation regressions: one pre-fix failure each; four filtered cleanup passes and one allocation pass after correction.
- Settled archive crate: **68 tests pass** (25 unit, 32 archive integration, 11 custody integration), including the existing SIGKILL, admission and staging cases.
- Strict crate Clippy: completed successfully in the supplied raw output.

The corrective slice adds five tests and reuses the existing subprocess helper. The serializer's two tests substantiate the bounded architecture; no serializer pre-fix run was supplied, and this report does not invent one. Recorded tool `wall_time_seconds` for completed/polled commands is not full execution time; raw build/test durations must retain their actual meaning. At review time, `attempt-metadata.json` covered the first three attempts; the integrator was asked to add exact known commands, exit codes and source attribution for the remaining logs before publication, with unmeasured fields explicitly labeled. The code GO does not assert that this metadata addition is already complete.

## Limits and handoff

The original full custody review remains the authority for the unchanged lifecycle/provenance scan. It found no further blocker. These corrections provide library custody/recovery and verified private staging evidence. Installed restore/recovery composition, production policy and P00 values, database/job binding, approvals/claims/service coordination, destination durability and effect-time recovery remain unfinished. BKP-001 and applicable P04 gates remain open. SIGKILL evidence does not establish actual power-loss durability; historical footprint evidence does not measure this corrected source.

The integrator owns merge, full exact-source gates, CI and final evidence publication. The reviewer only read source, docs and raw outputs and compared artifact data. No tests, repository/Git changes, host/SSH/Pi/wybie/Docker access, frozen Python work or delegation ran. This report is a new `/tmp` file; the original review report remains unchanged.
