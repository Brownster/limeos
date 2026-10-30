# Typed reservation for the retained mount journal

Source `a7bc867` accounts for the genuine journal already held by mount execution
and unmounted reconciliation. `inspect_with_journal` borrows that protected
`MountJournal`; its private capacity profile includes the journal's measured
five-descriptor bound. Execution, quiescent no-effect reconciliation and
readback of a precondition-changed receipt use this seam. Ordinary inspection
retains its existing profile.

| Bound | Ordinary inspection | Journal held |
|---|---:|---:|
| Baseline descriptor allowance | 16 | 21 |
| Full retained/transient reservation | 213 | 218 |
| Aggregate model ceiling | 226 | 226 |
| Service descriptor limit | 256 | 256 |

The journal lock stays held. Receipt replay still precedes fresh effect checks.
Four-second admission windows, poisoning, task/namespace/FD comparisons, helper
gating and uncertain-job barriers remain intact. A compile-fail case verifies
that a serialized receipt cannot supply the genuine journal parameter. The
resource profile is private and no request chooses a count or allowance.

The new isolated native capacity probe opens actual descriptors under a 256-FD
soft limit. Before the correction it failed at the execution baseline because
the journal had no reservation. With the correction, ordinary inspection
accepts 16 and rejects 17, while the journal profile accepts 21 and rejects 22.
The child releases its fixture descriptors; the parent verifies its own limits
were unchanged. This exercises capacity arithmetic; the separate genuine
journal regression verifies the measured five-descriptor owner footprint.

Failure-only diagnostics now retain the actual numeric capacity subcondition
for baseline, aggregate reservation, retained allowance, soft FD limit, RSS,
threads or sampled FD bound. These diagnostics use already-read values and
retain the existing refusal result.

Final-source checks passed:

- 49 executor library cases, 2 binary cases and 5 default integration cases;
  their isolated helper cases are ignored by the outer harness and explicitly
  run within the parent regressions.
- 7 executor and 13 storage compile-fail doctests.
- All 14 journal cases, including the actual owner descriptor regression.
- Strict executor/storage all-target Clippy, Rust formatting, 8 package-profile
  reference cases and repository checks.

The default executor run does not enable the separate `read-worker-probe`
feature. Unchanged kernel filesystem fixture tests were not repeated. All 687
recorded product inputs match the source commit. The original parent HEAD
`53eec20`, timestamps and raw output remain in each record. The red variant is
joined to that parent's source plus the exact capacity-test file. The separate
formatter procedure changed inputs intentionally and supplies no qualification;
its final hashes match the subsequent checks. Every recorded run completed
without the sampled 3-GiB memory stop. Scratch, caches and logs stayed on disk.

`source-match.json`, raw check records/output, exact source variants and harness
scripts are retained. `SHA256SUMS` covers every other packet file. The packaged
profile documentation is updated in the following documentation commit; these
native results retain their exact code-source identity.

This corrects resource accounting; it does not qualify a mount effect. Build a
new candidate and use fresh empty disks under continuous supervision to prove
actual mount, independent verification and replay/restart. Installed UI and the
wider A/B, ARM64, assistant and migration gates remain open. Holly's production
Pi was untouched and nothing was pushed.
