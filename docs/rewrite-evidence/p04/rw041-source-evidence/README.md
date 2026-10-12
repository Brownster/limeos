# Retained SnapRAID source local qualification

Base `1401c255d4a05b8621c2dcd497d6f7f0cbe91d56`.
Qualified runtime **`e0acec985895febd8d6173b450d0a465f30caafb`**.
Branch `engineer/snapraid-source-evidence`, worktree
`/tmp/limeos-snapraid-source-evidence`.

Read the [design](../../../plans/2026-10-12-snapraid-source-evidence-design.md)
and [API contract](../../../p04-snapraid-source-evidence.md). This is local
unregistered-library qualification with implementer source review. No independent
GO, remote CI, installed root/confinement success, ARM64, guest or performance
qualification is claimed.

## Actual checks

The exact-source full workspace passed **461 Rust cases**: 455 runtime/unit/
integration and six compile-fail cases; zero failed or ignored. Fifteen
substantive behavioral cases and two compile-fail cases are new. No subprocess
driver is counted among the additions. Compiler checks prove that serialized
reports cannot construct an owner and the owner cannot outlive its genuine
borrowed raw inventory.

Focused final tests, formatting, strict workspace Clippy, repository boundaries,
generated contract drift, Git whitespace, frozen-source and unchanged-boundary
checks passed. Cargo.lock, package versions and dependency edges equal the base.
Cached deny/audit pass for 227 packages (211 external, 16 local), with 1290
advisories at `ef6173cbc5c50ec8166f9a5b28f07834144373ee`, dated
`2026-10-03T10:14:03+02:00`. No advisory database was fetched; there is no online
freshness claim. Existing duplicate-version warnings remain in the deny log.

`qualification.json` binds counts, commands, resource observations, scope and
remaining gates. `source-manifest.json` binds 319 tracked runtime/build,
contract, fixture, script and packaging inputs, all byte-identical to the
qualified Git source. `runtime-diff.patch.gz` is the exact crate diff from the
base. Eighteen complete redirected logs/tool-version captures retain raw and
lossless gzip SHA-256 identities (gzip mtime zero). The packet's SHA256SUMS
covers every immediate file except itself.

```sh
sha256sum --check SHA256SUMS
```

A rerun uses the qualified source, an owned target/cache directory and recorded
commands. The full existing suite requires permission for its private Unix
listeners and crash-test children; the new tests create neither a guest nor a
SnapRAID/Docker process. No raw workstation storage collector is invoked.

## Real scratch versus synthetic facts

The new tests retain actual scratch directory/file descriptors, exercise
openat2/no-follow traversal and mutate real names, bytes and permission bits.
Their private stat adapter supplies explicitly synthetic device/mount identities
and UUID/boot/swap facts. It exists only in the test module and cannot construct
a public `StorageInventoryEvidence`. Thus the tests qualify descriptor logic
and the pure mount binder, not genuine successful protected host collection.

Existing single-disk/separate-downloads fixtures remain unconfigured and cannot
produce an empty successful source owner. The fifteen-drive protected pool
produces 33 ordered rows, 19 distinct paths and **35 additional retained FDs**;
the multi-parity case produces 12 rows, 10 paths and 15 FDs. Duplicate Source/Data
requirements share handles without dropping rows. These are fixture observations
with absent content/parity leaves; existing files, borrowed inventory and
transient traversal add costs. They are not universal memory, installed
headroom, real mount or performance results.

Regressions cover every generated role, missing/wrong-type directories,
symlinks, nonblocking FIFO refusal, hard-linked leaves, UUID ambiguity/replacement,
boot/swap, bind/read-only/unsupported/nested mounts, raw-table disagreement,
descriptor device/mount disagreement, real write/unlink/name replacement,
absence becoming presence, parent replacement, metadata changes, cancellation,
original local expiry, full configuration/limit identity, exact report limits,
closed limits records and successful/error-path FD teardown. No source file
content is read by the new collector; the genuine inventory retains raw probe
responsibility at the public boundary.

## Preserved attempts

`initial-check` failed because rustix StatxTimestamp lacks PartialEq. Comparing
its seconds/nanoseconds explicitly corrected the compiler error. `focused-first`
passed 11 cases and failed one setup: the fixture renamed a nested parent while
assuming it was the top-level mnt directory, so symlink creation returned
AlreadyExists. The correction renames the actual scratch mnt directory.
`focused-second` passed 12 cases; `focused-third` passed all 15 additions before
source freeze. No protection fallback or permission/cap relaxation was used.

Preliminary Clippy and test outcomes remain separate from exact-source final
qualification. Final source review also removed blocking path work after the
last original-inventory lifetime check; current ordering brackets paths with
raw inventory validation and ends with local clock checks only. `workspace`,
`focused-final` and `clippy-final` qualify that exact source. All failure logs
remain alongside the passing results.

## Ownership and remaining gates

Runtime changes are only the new protection_sources module/tests and one public
module export. Existing inventory, container sources/dependencies, domains,
generated contracts, core/schema/jobs/claims, protocol, packaging/workflows,
services/caps/UI and dependencies are unchanged. Main, before-images and active
backup reconstruction worktrees received no edits. The frozen Python repository,
Pi/wybie/SSH and workstation Docker were untouched.

The public caller still needs current protected configuration, authorization,
source/ancestor/ACL policy and writer exclusion. A report timestamp is not a
replacement for the original inventory's remaining lifetime or an effect-time
revalidation. No complete PoolProtectionReader implementation, status/diff
runner, sync/scrub, shared physical claims or storage effects are supplied.

Actual root/confinement collection, worker deadlines/reaping/admission,
whole-worker FD/memory headroom, pool/share propagation, diff freshness,
independent executor ceilings, sync/scrub and interruption recovery remain open.
**SRA-001, SRA-002, DSK-001, P04 effects and cutover stay open.** The historical
inventory p95 remains failed at 61.401 ms against 20 ms. Earlier guest refusals
and the 17 blocked combined cases are unchanged by local source-library tests.

Estimate 16 human hours, review at 24; P04 retains 320/480. Actual human and
active-agent effort are unmeasured. Build/approval/CI waits and elapsed calendar
time are not reported as human engineering hours.
