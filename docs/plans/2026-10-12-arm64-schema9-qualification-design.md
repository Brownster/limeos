# Current schema-9 native ARM64 qualification

Freeze runtime/source at `1401c255d4a05b8621c2dcd497d6f7f0cbe91d56`, whose
runtime `8288315` has passing native CI and 444 Rust cases. Use separate committed
fixtures from `engineer/arm64-schema9-qualification` in
`/tmp/limeos-arm64-schema9-qualification`. Do not include unmerged before-image,
SnapRAID evidence or active uncommitted recovery work.

The operator said the Pi is available tonight. Exact SSH target and current
start/end window have been requested and remain necessary before host access.
The old holly@wybie authorization expired October 7 and supplies no permission
for this run. Record fresh exact-target authorization with the existing guard;
never bypass it through another alias or invent a window.

Estimate 16 human engineering hours, scope review at 24. P04 stays 320/480.
Actual human and active-agent effort are unmeasured. Host/build/tool waits are
separate and cannot establish the phase's human-hours threshold.

## Local preparation

Extend only the ARM64 upgrade wrapper to admit genuine historical schema 8 as
well as 6/7 into current schema 9. Preserve the existing authority fixture's
sessions, approval bytes, verified receipts, complete uncertain claim sets,
queued state and exact replay checks. Validate previous source/version/schema/
architecture/package/core hashes before selecting or installing a package.
Require one contained regular nonsymlink artifact with exact SHA and controls;
missing/ambiguous/mismatched candidates refuse before the upgrade operation.

The genuine schema-8 package is retained in the original overnight worktree:
source `8acac403e32d3692b028a036a11f4fead2050580`, version `0.4.7+arm64.1`,
package SHA `6c86199b253e155c94d96e1395c069c4188e11a2379f14eff97eacab0203b898`,
core SHA `aab46a601a95924c8900a93749cd8cf5cc735cc2328783196732c363f258cde0`.
Reverify actual package bytes, controls, ELF/core and original build/evidence
bindings locally before preparing provenance. Never rebuild old source under
that label or count current CI's +ci.1 package as a historical upgrade.

Commit the fixture changes, run local guard/upgrade/harness regressions and
Python lint/format, then make a source/fixture/frontend-bound bundle with schema
9 and distinct package version `0.4.8+arm64.1`. The native guest build, source,
ELF, package, signed test repository and installed results remain separate
identities. Local mocked upgrade tests do not establish an installed upgrade.

Local preparation also reproduced a host-window defect: explicit authorization
was ignored outside the production host list. Apply any supplied record before
host resolution, including on isolated targets. Preserve the default production
refusal and the existing per-dispatch timeout and supervised deadline rules.
Require regression proof for exact-target binding, invalid/expired windows,
deadline calculation and refusal of dispatch after cutoff.

## Tonight's priority

1. Read-only host capacity/architecture/KVM/image preflight within the recorded
   window. Prove owned host supervisor deadline and unrelated-process preservation
   with a short guest before starting the long run.
2. Build the frozen source natively with Rust 1.88, locked dependencies, required
   source gates and standard/shadow packages. Preserve complete build/package
   and signing proof. One guest, nice 19/idle I/O, at most 3 CPUs/2048 MiB.
3. Fresh standard/shadow install and footprint: PSS/RSS/swap, ten-minute idle CPU/
   writes, readiness, authenticated latency and installed bytes. At most 2 CPUs/
   1536 MiB; reduce under host pressure. Native KVM is not bare metal or a matched
   Python comparison.
4. Genuine schema-8-to-9 upgrade, then existing storage/container interruption
   suites if the window permits. Synthetic disks only, with existing serial,
   size, boot and mount guards; no host folders or physical disks passed through.
5. Collect raw outcomes, failures, journal and host pressure, stop/reap every
   owned guest before the exact cutoff and record teardown. Mark every omitted
   suite untested. No retry or guest launch after the window expires.

Own only `tests/qualification/arm64/`, this design, a current runbook and new
ARM64 evidence. Keep main, runtime/schema, shared authority fixtures, service
policy, capabilities, installed host packages/groups, production Docker/Python,
mounts and data unchanged. Harness preparation does not close native/installed,
backup, P04 or cutover gates. Inventory p95 61.401 ms against 20 ms remains
failed until a separately scoped new result is measured.
