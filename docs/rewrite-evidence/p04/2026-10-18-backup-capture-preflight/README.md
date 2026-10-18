# Before-image capture preflight — local evidence

Tested product source: `bf9a90959924617b36fe37022f22dde8ef7dad00`, branch
`engineer/p04-backup-capture-preflight`, based on main
`7df3f7530865c31b7faefd1f7b12a25aa7655860`.
The [handoff](../../../p04-backup-capture-preflight.md) records API semantics
and the unfinished central admission/runtime requirements. This packet covers
the actual archive crate with its unchanged dependencies and local scratch
fixtures; it does not qualify an installed backup or restore coordinator.

| Check | Result |
| --- | --- |
| Before implementation: existing before-image library filter | 18 passed, 57 library cases filtered; integration suites filtered out |
| Final complete archive library | 87 passed |
| Archive admission integration | 32 passed |
| Before-image integration | 2 passed |
| Custody integration | 11 passed |
| Private preparation integration | 11 passed |
| Compile-fail documentation | 5 passed |
| Final total | 148 checks, zero failed/ignored |
| New coverage included in total | 12 unit cases plus one non-deserializable-plan doctest |
| Existing maximum-object fixture, repeated with output | Passed; planning added zero FDs, capture retained 258 |
| Strict all-target Clippy | Passed |
| Changed-source Rustfmt and `git diff --check` | Passed |

The full final command was `cargo test --offline --locked -p
limeos-backup-archive`. Clippy used `cargo clippy --offline --locked -p
limeos-backup-archive --all-targets -- -D warnings`. Rustfmt used edition 2024,
`--check --config skip_children=true`, and the three changed Rust paths. Test
execution used one Cargo job, one test thread, disabled incremental compilation,
the shared repository target cache and repository disk `TMPDIR`; the actual
settings are in the manifest. No simultaneous local build was observed when
starting or checking this work.

The separate maximum-object measurement runs the already-counted fixture with
`--exact --nocapture --test-threads=1`. It forks an isolated child so other test
threads cannot distort descriptor counts. Both parent and child report the
same single test; neither adds another case to the total. Its 256 selected
files contain 2,048 logical bytes. Planning retains no added descriptors;
the completed before-image owner adds 258 and its canonical record occupies
152,862 bytes in this fixture. The one transient descriptor allowance comes
from reopening/listing in the actual code, rather than a measured process
peak. No complete worker FD, RSS, task, physical disk or cgroup claim follows.

New cases exercise original bindings, offsets and costs, decline/drop without
creation, canonical record/digest compatibility, changed source sizes/names/
modes/parent/last-object metadata, root protection and pathname replacement,
cancellation, limit/duplicate refusal and preserving foreign orphan objects.
Refusal cases use an actual attempt-creation checkpoint counter as well as
checking private names, so successful cleanup cannot masquerade as preflight
refusal. Existing copy/hash/short-I/O/fsync/cleanup and scratch-process crash
checks remain in the full crate suite. These are not live restore, SIGKILL of
an installed worker, power-loss or physical consistency tests.

The first offline baseline failed dependency resolution before compiling:
the workspace lock selected Rustls 0.23.46, absent from the cached index for
the unrelated footprint spike. An authorized exact-lock online fetch enabled
the 18-case baseline and all later offline checks. Neither Cargo.lock nor
any manifest changed. The failed initial log is retained. The baseline filter
is not credited as a full old-crate run.

`inputs.sha256` binds the archive and domain sources/manifests, actual backup
fixtures, root Cargo.lock/workspace/toolchain and tracked Cargo settings.
`changed-paths.txt.gz` contains only the three A-owned Rust paths and the
committed design; all shared roots, Store/schema, protocol, worker budgets,
policy, routes, services and package inputs are unchanged from the base.
The handoff/evidence are separate documentation commits after the product.
`artifacts.sha256` binds this packet's raw compressed logs, manifest, scripts,
source list and exact lockfile copy. Inputs were hashed after the reviewed,
unchanged tested Rust changes were committed; no new source edit followed
the recorded successful test/Clippy/format runs.

The central worker must admit source collection itself, then combine the
opaque plan's component costs with every simultaneously held caller/root/
catalog/custody/prepared/snapshot/transport and baseline cost, and reserve the
whole operation before capture. The 256-FD service cannot accept an owner
requiring 258 retained descriptors alone. This module does not reserve or
raise that ceiling. Metadata preflight does not freeze bytes or supply current
authorization, physical claims, writer exclusion, online database snapshots,
durable publication or recovery authority.

Encrypted creation/provider/key recovery, complete managed inventory/media
exclusions, installed capture/publication/restore and central admission remain
open A4/A5 requirements. No hosted CI, package/VM/ARM64/Pi/hardware/resource,
performance, installed recovery, defect, P04 or cutover gate closes here.
The historical storage inventory 61.401 ms p95 against 20 ms remains failed.
No production host, service or live destination was touched.

Verify packet artifacts from this directory with `sha256sum -c artifacts.sha256`.
Verify inputs from the worktree root using this packet's `inputs.sha256` path.
