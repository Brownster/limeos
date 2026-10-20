# Completed local main validation, 20 October 2026

Qualified source: `6466eb0236cf4599bd6c67555277b1a8cdccfc1b`. Its product bytes match
the assembled source `b587621` recorded in the
[initial integration packet](../2026-10-19-local-main-consolidation/README.md).
Later commits change documentation only. The [handoff](../../../p04-local-main-consolidation.md)
records the integrated scheduling/ABI work and remaining runtime boundaries.

| Actual local check | Result |
| --- | --- |
| Workspace excluding executor-storage | 906 unit/integration/doctest checks passed |
| Executor-storage unit suite | 211 passed |
| Executor-storage compile-fail doctests | 13 passed |
| Complete workspace across those three disjoint runs | 1,130 passed, none failed/ignored/filtered |
| Combined Stream B harness | 507 passed: 154 adapters, 41 API, 306 domain, six runner |
| Strict all-target workspace Clippy | Passed with `-D warnings` |
| Strict all-target combined B Clippy | Passed with `-D warnings` |
| Generated-contract drift | Passed through the actual Rust generator |
| Unchanged frontend in earlier packet | 219 cases, strict TypeScript and production build passed |
| Cleanup | 78 clean merged worktrees and 87 merged local branches removed; eight dirty worktrees retained |

Root API cases overlap the workspace and B harness. These totals remain separate;
they are not a claim of 1,637 unique tests. Repeated runs add no coverage. The
earlier packet separately records 57 reference and 63 ARM64 harness checks.

## Filesystem fixture setup

The first complete native attempt reached the tests and failed 17 existing
storage cases with disk scratch on Btrfs. Its complete raw outcome remains in
`initial-btrfs-run/`. The directory-link test assumes a changing directory link
count; source tests also meet the unchanged mount/device identity refusals on
this filesystem. No production filesystem check or test assertion was relaxed.
This record does not qualify Btrfs source resolution.

The established [storage fixture procedure](../2026-10-13-storage-configuration/README.md)
compiles on disk before executing tiny owned tmpfs fixtures. This run follows
that pattern using a private `/run/user/1000/limeos-storage-fixtures-*` directory,
not `/tmp`. Cargo, compiler output, logs, UV caches and general scratch remain on
repository disk. The runtime helper selects the actual Cargo compiler artifact,
hashes it before/after execution, applies a 16 MiB per-file limit and a 16 MiB
sampled aggregate stop threshold, then removes its private directory.

All 211 storage tests pass; sampled fixture data peaked at 57,496 bytes. The
directory was verified absent after cleanup. The threshold is a sampling guard,
not an atomic aggregate-memory admission proof. No mount, format, live service
or production disk effect was performed. The original 3 GiB available-memory
guard, one Cargo job and one test thread remained in use.

## Reproduction and binding

`manifest.json` records exact commands, toolchain, kernel, component counts,
headroom observations, runtime fixture identity and cleanup totals. Each guarded
check retains before/after source hashes and raw output. All final check records
bind source `6466eb0`, return zero and show unchanged inputs. Build dependencies
were reused offline under the unchanged production lock. The B harness was
regenerated from the tracked entry point and seeded with that lock; exact
generated wrappers and final separate lock are under `harness/`.

`inputs.sha256` is the same 669-file conservative source/configuration/fixture
manifest as the earlier packet and verifies from local main. It does not imply
every hashed fixture was used. `artifacts.sha256` covers this packet. Capture
scripts retain their actual execution paths and source checks; adjust those
paths and restore the recorded source when reproducing on another checkout.

Cleanup retained ignored evidence caches and complete Git recovery bundles in
`.cache/main-consolidation/`. Its raw outcome and original ref identities remain
available. Every remaining local branch HEAD is included in main; the eight
remaining worktrees contain uncommitted drafts and were not discarded or credited
as accepted runtime work. No remote branch was pushed or deleted.

Fresh native ARM64, installed package/VM, hardware, performance, crash recovery,
P04 completion and Pi cutover are not claimed. The temporary CI publishing
question is optional for this completed local consolidation and remains unacted
upon without the user's permission.
