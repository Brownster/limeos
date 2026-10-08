# RW-043 private restore preparation handoff

Base: `2fd74209e1238c1374837ab431ef670020726dc2`.
Qualified source: `92a149dd24188912ce56581bc012e09f70c29f7d`.
Branch: `engineer/backup-restore-preparation`.
Worktree: `/tmp/limeos-backup-restore-preparation`.

The required baseline full CI, run
[37746766303](https://github.com/Brownster/limeos/actions/runs/37746766303), was
observed completed/successful on exactly `2fd74209` before worktree creation.
This handoff has local qualification; native/package/installed integration CI
belongs to the integrator.

## Delivery and source boundaries

The [API/lifecycle contract](../../../p04-backup-restore-preparation.md) describes
the immutable genuine-catalog plan, closed trusted targets, independent private
copies, metadata facts, canonical records, borrowed readers and cleanup.

| Commit | Scope |
|---|---|
| `9f63a68` | Isolated read-only catalog revalidation seam; sealed metadata and fresh name/size/type/hash checks |
| `fe1f22b` | Required one-line module export with a compileable placeholder |
| `276dc00` | Private preparation library and genuine-catalog regressions |
| `92a149d` | Preserve underlying cleanup listing/name-check I/O errors |

The initial brief reserved staging to the integrator. The pinned reader hides
appended bytes and retained/name identity changes. A concrete read-only API
proposal was sent; the operator replied `continue`. The implementer stated that
this was being treated as approval for the isolated seam. **The integrator must
review that separate shared change**, rather than treating it as the original
one-line allowance. No other shared source changed except `pub mod preparation;`.
Custody, its three reviewed corrections, inspector/decoder, domain, lockfile,
old fixtures/evidence, API/IPC/CLI, jobs/claims, packaging, workflows and UI remain
byte-identical to the base.

[The owned-path patch](allowed-path-diff.patch.gz), [source manifest](source-manifest.json)
and [qualification metadata](qualification.json) bind 248 source/fixture/build
inputs and the preserved logs to the qualified source. The patch is the runtime
diff; documentation/evidence have their subsequent commits. The recorder refuses
unexpected paths or changed tested inputs. Generated gzip/zstd scratch fixtures
live in the committed Rust tests; no production mapping/default is inferred.

## Checks

| Check | Result and retained output |
|---|---|
| Final workspace tests | **399 unit/integration + 4 compile-fail = 403**, zero failed/ignored; [raw output](workspace-92a149d.txt.gz) |
| Focused private fault/crash/bounds tests | 18 pass, including one subprocess helper; [output](preparation-92a149d.txt.gz) |
| Formatting | Pass; [output](fmt-92a149d.txt.gz) |
| Strict workspace/all-target Clippy | Pass with `-D warnings`; [output](clippy-92a149d.txt.gz) |
| Repository boundaries | Pass; [output](repository-92a149d.txt.gz) |
| Diff whitespace | Pass; [output](diff-check.txt.gz) |
| Generated contracts | Pass; [output](contracts.txt.gz); unchanged inputs are source-bound |
| Offline cargo-deny | Pass; [output](deny-unrestricted.txt.gz), three existing duplicate-version warnings |
| Cached cargo-audit | Pass; [output](audit-final.txt.gz), 1,290 advisories, 227 packages |
| Evidence recorder | Executes successfully; Ruff check/format pass on the new helper |

Relative to the 370+2 baseline, there are **29 new unit/integration cases**
(28 substantive, one child helper) and **two new compile-fail cases**. The helper
is counted once and identified separately, not presented as an independent
acceptance scenario. Focused and full totals overlap and are never added.

Advisory checks did not fetch. Cached database is
`ef6173cbc5c50ec8166f9a5b28f07834144373ee`, dated
`2026-10-03T07:49:26Z`; no latest-advisory claim is made. All 211 external
version/source/checksum identities among 227 packages remain unchanged, with 16
local packages and no new dependency/edge/version. No package or frontend build
was repeated for this unregistered library.

## Meaningful scenarios

| Scenario | Proof |
|---|---|
| Exact gzip/zstd bytes, empty and 64 KiB boundary payloads | Genuine admission/replay; independent rereads; deterministic complete records and trusted installed facts |
| Fresh custody recovery | Real retain/drop/recover/fresh stage, then preparation; independent copies survive catalog/custody discard |
| Unknown/duplicate/missing resources/targets, metadata and manifest changes | Whole plan/preparation refuses; no object appears |
| Traversal, symlink/magic-link roots, wrong type and overlap | Protected root/closed target refusal; no live or foreign write |
| Catalog append and name replacement | Original reader behavior retained as diagnostic; new validation refuses both before owner exposure |
| Mutation after copying/root barrier | Final catalog revalidation refuses and removes only verified private objects |
| Short/interrupted/zero/corrupt writes, ENOSPC, independent short reads | Private I/O injection against actual scratch copies; exact output or typed whole refusal |
| Allocation/overflow, limits, each fsync barrier | Fallible reservations/capacity overflow and file/directory/attempt/root faults; no owner |
| Primary plus cleanup errors | Separate ENOSPC and fsync/listing EIO facts retained |
| Unverified creation, replaced file/attempt, unknown children | Foreign/replaced names are retained with explicit cleanup facts |
| Cancellation | Every lifecycle point, including after file/root barriers, returns no owner |
| Reader mutation and lifetime | Fresh checks and failure latch; compile-fail owner import and reader lifetime |
| SIGKILL | Real owned subprocess at creation, file sealing, attempt sync and root sync; reaped; inert orphan preserved; fresh attempt does not adopt it |

Managed bytes and unrelated sentinels remain unchanged throughout. There is no
rename/publication in production preparation, so a preparation rename-fault
claim is inapplicable. Replacement tests rename only owned scratch objects to
inject races.

## Bounds and measured scope

Maximum policy: 64 resources, 256 total objects, 4096-byte/32-component paths,
255-byte components, per-file and aggregate logical bytes at most `i64::MAX`,
and 16 MiB canonical records. Limits must be explicit and positive. The
256-object fixture has 255 eight-byte files plus one required directory:
**2,040 logical payload bytes, 144,172 record bytes and 258 additional retained
descriptors**. One additional descriptor is transient; staged catalog/caller
state is separate. This is a small-payload FD/record case, not maximum-disk or
universal heap/performance qualification. The contract records the possible
combined `F + N + 6` descriptor requirement and future admission obligation.

Disk addition is selected payload bytes plus filesystem metadata; custody and
all staged bytes have separate costs. No disk reservation or global quota is
provided. Deterministic allocation/ENOSPC/fsync injections are distinct from
real scratch and SIGKILL tests. No forced real filesystem exhaustion,
process-wide OOM survival, power-loss durability or installed recovery is
claimed.

## Retained failures and infrastructure

Earlier outcomes remain separate from final acceptance:

- Initial diagnostic compile typo (`snapshot` vs `policy_snapshot`) and format
  output are **tool-chunk excerpts**, not a complete compiler transcript;
  terminal whitespace was not preserved when those excerpts were copied.
- Initial broad sandbox tests refused temporary Unix listeners; offline deny
  refused a read-only advisory lock. Required checks were rerun with access.
- The first private-test build hit sandbox disk quota; its full output is kept.
  The owned 2.8 GiB target cache moved from tmpfs to the repository's ignored
  `target/backup-restore-preparation` directory.
- One integration-test compile had a duplicate mutable borrow; the corrected
  fixture uses a separately computed duplicate selection.
- Relocating the cache left the password-worker test's compiled-in old path.
  [Diagnosis](cache-path-diagnosis.txt.gz) proves it; only the owned core package
  cache was cleaned. The corrected workspace run passed without source changes.
- First strict Clippy found a redundant closure. Source review later found
  cleanup I/O flattening, corrected in `92a149d` with the new regression.
- The evidence helper's initial Ruff findings (executable bit/import ordering)
  and corrected passing outputs are retained. A first Ruff command was absent
  from PATH; an already-cached Ruff 0.16.10 executable was used without fetching.
- The daemon restart interrupted command launch after the cache move. State
  was checked before resuming; no completed source change was repeated.

Complete redirected output is compressed losslessly with round-trip checks.
`qualification.json` records compressed and original-byte hashes. `SHA256SUMS`
covers this packet and excludes itself.

## Remaining gates and effort

Existing-target before-image capture is deferred; caller-authorized retained
managed descriptors and independent snapshots are still needed. No pathname or
hard link is presented as recovery data. **BKP-001 stays open.** Production
policy/limits, durable job/approval binding, shared claims, independent worker
admission/deadlines, snapshots/service quiescing, live replacement, installed
metadata, uncertainty journals, receipts and installed recovery/power-loss
rehearsal remain integrator work. No P04 defect or cutover gate closes and the
historical inventory row remains failed at 61.401 ms against 20 ms.

Assignment estimate stays 24 human hours/review at 36; P04 stays 320/480. Human
hours and active agent time were not independently instrumented. Approval and
interface waits, compilation/cache work and daemon interruption are separate
from human effort. Tool wall intervals include long waits and are not summed
into engineering hours. No SSH/Pi/wybie, VM/production operation or frozen
Python change ran. Only the feature branch is committed; main publication and
merged qualification remain with the integrator.
