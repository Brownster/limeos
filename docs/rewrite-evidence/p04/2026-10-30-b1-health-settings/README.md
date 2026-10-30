# Bounded frozen health-settings migration

Source `32265d4` implements a pure selected-settings planner. A successful result
contains a complete representable health-policy candidate; a refusal returns
only a bounded field/reason/index. It supplies no persistence or execution
authority. The [handoff](../../../p04-b1-health-settings-migration-handoff.md)
describes the explicit compatibility profile and remaining integration.

The final domain run passes **559 runtime tests and seven compile-fail
doctests**, zero failures or ignored tests. Twelve new tests cover all 21
applicable daemon/policy/silence-expiry/policy-file precedence cases in the
existing pinned 32-case fixture. Frozen source identity and fixture hashes
remain unchanged. The other eleven cases concern external selection,
provisioning and preferences, which are outside this planner.

Boundary tests distinguish malformed/defaulted integer input from valid
unsupported runtime values; preserve explicit empty policy, literal path case,
exact-only deduplication, category settings and silence metadata; and refuse
invalid structures and excessive bytes/depth/row counts without truncation.
Original creation text and fractional precision survive. Nonzero fractional
expiry refuses rather than being rounded. Calendar/offset conversion matches
independently calculated Python datetime constants, including leap-year and
epoch limits. The actual `fold_alert` retains a muted open incident and announces
it exactly at the exclusive expiry instant.

Strict all-target domain Clippy, actual contract regeneration, Rust formatting,
repository hygiene and diff checks pass. The first Clippy attempt refused one
nonminimal boolean expression in the integer syntax predicate. Its original
output remains alongside the corrected final run. The earlier passing Rust run
differs only in that expression's source bytes; the final suite qualifies the
committed simplification. Earlier results are not added to final test counts.

The final Rust, Clippy and contracts guards contain identical before/after maps
for all **702 committed source inputs**. Each hash was compared with the actual
committed blob and current file. The recorded HEAD is the preceding design
commit because validation preceded the feature commit; content hashes establish
the join. `source-match.json` preserves this attribution. No generated contract,
dependency, SQL schema, runtime registration or UI source changed.

All five guards completed with unchanged inputs, no external compiler and no
sampled memory stop. Final Rust ran 22:16:14–22:17:08 UTC on 30 October; its minimum
host availability was 4,279,930,880 bytes, above the unchanged 3-GiB runtime floor.
Compilation starts required 4 GiB available. Checks used the pinned Rust 1.88.0,
one Cargo job, one test thread and repository-disk scratch/cache/logs. No host
`/tmp`, deployed settings, production Pi, package/service activation, ARM64,
external send or push was used.

`checks/` contains original compressed guard records and full logs for both Rust
runs, both Clippy attempts and final contract regeneration. `launch-and-local/`
contains original admission/launch records, local checks and tool versions.
`reproduction-sources.tar.gz` retains exact changed source, the existing frozen
fixture/reference harness and detached validation drivers. `source.patch.gz`
retains the feature commit. `SHA256SUMS` covers every other file.

This qualifies a migration prerequisite. Protected actual-state acquisition,
durable policy/silence metadata, current policy/destination revisions through
leases/completion/publication, concrete resource replacement, genuine collection,
settings API/UI, supervision and installed restart/idle-write evidence remain
open. It does not close B1, ordinary external parity, A1 reliability, P04 or the
P05/P06 Pi cutover gates.
