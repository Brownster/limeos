# Completed rewrite slice integration

Qualified source `016ba49a27b228576db5d4ca28dcd122c60a6278`, based on main
`1401c255d4a05b8621c2dcd497d6f7f0cbe91d56`. The operator requested integration
and push of completed work on October 12. Each delivery retains its original
commits/evidence and a separate merge commit:

| Delivery | Branch tip merged |
| --- | --- |
| Verified private backup before-images | `d06f98494288510c4c35f13f6538fce5cd5e2908` |
| Retained SnapRAID source evidence | `c95bcdd1f1cd16e3f158087a14a5e1733a804e09` |
| Declared pool/protection dependencies | `eacd764de470040605312faeddeff3e807ad6966` |
| Checked Samba share planning | `cebf2dc757508f5c5b250035d81bdd95fdfa9bc9` |
| Current ARM64 qualification harness | `5efc3691baeaf5db5c5e6a3aa115d0fe6b884387` |

The only merge conflict was adjacent additive domain exports; both modules are
retained. The dirty `integrator/p04-backup-bound-recovery` worktree is excluded
and unchanged. No unfinished recovery code is qualified here.

Combined acceptance passes **521 Rust cases**: 513 unit/integration cases and
eight compile-fail doctests. The disk-scratch workspace excluding executor-storage
passes 420; executor-storage passes 101 with small auto-cleaned tmpfs fixtures.
Strict workspace/all-target Clippy, fmt, contract drift and repository checks
pass. The merged ARM64 harness passes 41 local tests. Frontend install/test/build
passes against the frozen archive when making the ARM64 input bundle. Native CI
and installed ARM64 results are separate evidence, not supplied by these checks.

Integration exposed a process-wide FD measurement race in the before-image unit
test. The original source measured 258 retained descriptors in isolation, 255
with two threads and 468 with eight. Concurrent test owners change `/proc/self/fd`.
Commit `016ba49` follows the existing process-evidence test pattern: the exact
measurement runs in an owned single-test child, retaining the 258 assertion and
adding exact release after discard. The full archive test binary passes with
eight threads and the workspace passes with two. Production capture code and
resource limits are unchanged. [Pre-fix records](pre-fix.json) and raw failure
transcripts retain the reproduction; acceptance totals count final runs only.

[checks.json](checks.json) records commands, environment, exits and scratch
measurements. [source-manifest.json](source-manifest.json) verifies 290 qualified
source/fixture/contract/script hashes against the exact Git source. Historical
engineer packets continue to refer to their original snapshots. Their former
unmerged/branch status does not describe this integration. The retained capture
script documents local paths and the sampled temporary-file size calculation.
Raw transcripts keep their terminal blank lines. Hashes in `evidence-sha256.json`
cover this packet except the index itself.

All worktrees, compilers, caches, general scratch and logs stay on disk under
`/home/marc/Documents/github/lime-os`. Only existing executor-storage fixtures
use the owned tmpfs directory recorded in the check metadata, then remove it.
Those fixtures require stat/mount-table and directory-link behavior that the
workstation's Btrfs scratch did not qualify. No source guard was weakened.

The new [integrated Pi runbook](../../../plans/2026-10-12-arm64-integrated-qualification-runbook.md)
supersedes the schema-9 preparation's earlier candidate for tonight's tests.
The candidate label is `0.4.8+arm64.2`, schema 9, and both source/fixtures freeze
at `016ba49`. Bundle SHA-256 is
`fc746b8e564c015ce7bf6757ad60d833b5fd6cb88ff028c376bef3109c9414a3`.
Its 1793 source, 39 fixture and three frontend-dist files were verified after
disk-backed extraction. The genuine schema-8 package remains byte-identical:
`6c86199b253e155c94d96e1395c069c4188e11a2379f14eff97eacab0203b898`.
The external manifest and [bundle verification](bundle-verification.json) retain
the source identities. No ARM64 native package was built or installed locally.

No Pi/SSH target or window has been supplied or accessed. Existing production
windows remain expired. Hardware tests await the exact operator target and
quiet-window record. New work starts from pushed main on a separate branch and
cannot silently alter this candidate's source identity.

This delivery supplies library planning/evidence/preparation and qualification
tools. It registers no additional live share, pool, SnapRAID, restore or mount
operation. Protected dependency composition, complete claims, approvals,
effect-time locks/revalidation and recovery remain integration gates. DSK-001,
MNT-001/MNT-002, RT-001, SRA-001/SRA-002, BKP-001, UI-003, P04 and cutover remain
open; inventory p95 61.401 ms versus 20 ms is unchanged. Phase effort remains
unmeasured; P04's 320-hour estimate/480-hour review are unchanged.
