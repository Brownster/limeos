# Declared pool/protection dependency acceptance

Implemented and locally validated on `engineer/pool-protection-dependencies`.
Qualified code: `356f52849a0f48d1fdd5d1b0c31e57a08d8924bf`, based on
`1401c255d4a05b8621c2dcd497d6f7f0cbe91d56`. Worktree:
`/home/marc/Documents/github/lime-os/.worktrees/pool-protection-dependencies`.

The pure domain planner maps declared container sources through configured pool
roots to every member device, alongside the existing SnapRAID planner's source,
data, content and parity requirements. It preserves numeric media ownership,
literal paths and branch order, volume identity, each destination, writable and
running facts, disabled configuration and the original declaration timestamp.
Invalid/incomplete configuration or declaration facts refuse; a 513th expanded
container match refuses the entire request. No result is clipped.

[Interface and limitations](../../../p04-configured-storage-dependencies.md),
[design](../../../plans/2026-10-12-pool-protection-dependencies-design.md) and
[case matrix](case-matrix.md) define the scope. This adds one module/export,
its tests and a dedicated golden fixture, with no new crate dependency or change
to existing planners, contracts, schema, dispatch, API/IPC/CLI/UI or service caps.

## Source-bound validation

| Check | Result |
| --- | --- |
| New pure planner regressions | 16 pass across four frozen layouts and refusal/propagation cases |
| Workspace excluding storage crate, disk scratch | 376 checks pass: 372 unit/integration + 4 compile-fail |
| Storage crate, small owned tmpfs fixtures | 84 checks pass: 82 unit + 2 compile-fail |
| Combined source coverage | 460 checks: 454 unit/integration + 6 compile-fail; no failures/ignored cases |
| Strict workspace/all-targets Clippy | Pass |
| Formatting, contract drift, repository checks | Pass |
| Frozen Python fixture provenance | All 17 source hashes match `80593b29443ea9b81eb60ed4e5c3c3ac7a236204` |

Complete final logs are `logs/test-disk-rest.txt`, `logs/storage-fixtures.txt`,
`logs/clippy-disk-final.txt` and `logs/contracts-disk.txt`. [Checks](checks.json)
record commands, environments and counts; [source hashes](source-manifest.json)
bind Rust/Cargo, fixture, schema and check inputs. The frozen Python files were
read via Git objects only. There was no Pi, SSH, production Docker or frozen
Python execution/development.

The first focused compile found a test-only conflicting borrow, corrected before
the 16-test acceptance. The first workspace attempt hit the sandbox compiler
cache restriction; its authorized retry then hit the tmpfs quota before tests
ran. Those complete attempts remain in `logs/limeos-pool-dependencies-*.txt`.
The first strict Clippy run rejected boolean `then` inside `filter_map`; the
equivalent `filter`/`map` form passes. No warning allowance was added.

## Disk relocation and filesystem limits

At the operator's request, active work and the build cache moved from `/tmp`
to disk. The active worktree's copied files, HEAD, branch and staged/unstaged
diffs matched before original removal. The 6,303 cache files were hash-verified
(3,060,202,218 apparent bytes; about 2.6 GiB allocated). Other completed branches
were relocated during concurrent workspace cleanup and their recorded HEADs and
clean states were confirmed; this packet does not claim our script copied them.
[Relocation record](relocation.json) gives the actual destinations. Across the
cleanup, `/tmp` usage fell from 13 GiB to 1.7 GiB and available RAM reached about
17 GiB. General scratch, UV/compiler/build caches and logs are now disk-backed.

A complete all-disk test attempt recorded 15 existing storage failures. The
workstation's Btrfs scratch has directory link count 1 and a stat device differing
from its mount-table device; the unchanged identity checks refuse these fixtures.
[Filesystem facts](filesystem-facts.json) and `logs/test-disk.txt` preserve this.
The same isolated source test passes with tmpfs. Full validation therefore uses
disk scratch for the general suite and an owned, auto-cleaned tmpfs scratch for
the storage suite. It passed all 84 storage checks, with sampled scratch-file
peak 912,880 bytes (about 892 KiB), then removed the scratch directory. This is
an observed fixture sample, not a universal process-memory bound or Btrfs backing
qualification. No production identity check was weakened or failing test skipped.

## Remaining integration

The result is declared-only. It retains no filesystem/namespace owner, physical
UUID/backing proof, storage lock, claim or approval. Empty relations cannot prove
safe unmounting: aliases, symlinks, named volumes, nested mounts and running
namespace paths need protected resolution. Shares remain an explicit missing
input. Integration must compose complete fresh owners with their original
lifetimes and bind full configuration, claims and approval before effects.

DSK-001, MNT-001/MNT-002, RT-001, SRA-001/SRA-002, BKP-001, P04 and cutover remain
open. The prior inventory p95 61.401 ms/20 ms failure stays failed. Estimate 12
human hours, slice review 18; P04 remains 320/480. Actual human/agent effort is
unmeasured; tool/build/approval waits do not establish the human-hours threshold.
The current Pi qualification bundle retains its independent frozen identity.

[Evidence hashes](evidence-sha256.json) cover this packet excluding the index
itself. Source code and documentation/evidence are separate commits for review.
