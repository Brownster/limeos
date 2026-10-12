# Private before-image local qualification

Base: `1401c255d4a05b8621c2dcd497d6f7f0cbe91d56`.
Qualified runtime: **`c6ac7fbcb632222b685a548a3be245d2d066b29c`**.
Branch: `engineer/backup-before-images`.
Worktree: `/tmp/limeos-backup-before-images`.

The [design](../../../plans/2026-10-11-backup-before-images-design.md) and
[API contract](../../../p04-backup-before-images.md) define the supported
metadata and caller trust boundary. This is local private-library qualification,
with implementer source review. No independent GO, installed worker, live
restore, power-loss, native ARM64 or Pi qualification is claimed.

## Actual checks and binding

The full exact-source workspace run passed **466 Rust cases**: 460
runtime/unit/integration and six compile-fail cases; zero failed or ignored.
Twenty runtime cases are new, including one ordinary-return subprocess driver;
nineteen are substantive behavioral tests. Two compile-fail cases prove that
serialized metadata cannot construct an owner and a reader cannot outlive it.
The focused final run passes 18 module cases, including that driver. The two
external integration cases also pass in the full run.

The genuine gzip/zstd case constructs actual admitted catalogs and private
replacement owners, then independently captures different existing target
bytes and metadata. Discarding catalogs/replacements and closing source handles
does not invalidate the before-images. Live scratch target bytes remain intact.
The other integration case uses the existing single-disk, separate-downloads
and fifteen-drive protected-pool configuration fixtures and preserves every
configuration byte, including their numeric ownership data. These fixtures do
not establish installed filesystem ownership or production policy.

Formatting, strict workspace Clippy, repository boundaries, generated contract
drift, Git whitespace and frozen-source/boundary checks passed. Cargo.lock,
package versions and dependency edges are byte-identical to the base. The
cached deny/audit checks pass for 227 packages (211 external and 16 local).
The advisory cache contains 1290 advisories at
`ef6173cbc5c50ec8166f9a5b28f07834144373ee`, dated
`2026-10-03T10:14:03+02:00`; no database was fetched and no online freshness
claim is made. Deny retains the existing duplicate-version warnings.

`qualification.json` records actual counts, commands, platform, limits and
remaining gates. `source-manifest.json` binds 321 tracked runtime/build,
contract, fixture, script and packaging inputs; every byte equals the qualified
Git source. `runtime-diff.patch.gz` is the exact base-to-runtime crate diff.
Nineteen complete redirected logs/tool-version outputs retain their raw and
compressed SHA-256 bindings. Compression is lossless gzip with mtime zero.
`SHA256SUMS` covers every immediate packet file except itself.

```sh
sha256sum --check SHA256SUMS
```

For a rerun, check out the qualified source, supply an owned Cargo target/cache
and use the recorded commands. The full suite needs permission to create its
owned local Unix listeners and test children; it does not contact workstation
Docker. The required compiler/cache environment is recorded in the JSON.

## Resource and crash evidence

The exact-source focused run's maximum fixture captures 256 regular files,
2048 logical bytes and a 151326-byte record. It retains **258 additional
descriptors**; caller source/parent/root, custody, catalog and replacement owners
are separate. This is a fixture observation, not an RSS/heap/block-space bound
or installed admission proof. It exceeds a 256-FD ceiling before other owners;
future composition must admit or refuse the whole request within unchanged
service limits.

The owned subprocess is SIGKILLed and reaped at AttemptCreated, Sealed,
AttemptSynced and RootSynced. Each leaves an inert original-inode orphan, with
no published record. A fresh capture creates a separate owner, leaves the orphan
unadopted and discards only its new attempt. Sentinels/current target bytes
remain intact. No real power-loss or installed orphan collection is exercised.

Private fault tests cover short/interrupted I/O, zero writes, ENOSPC, corrupt
copy verification, all four object/attempt/root durability barriers, allocation
refusal, cancellation, late source/name/metadata/directory changes and private
reader failure latching. Cleanup retains foreign/replaced/unverified objects
and preserves primary ENOSPC plus cleanup EIO without a silent Drop retry.

## Preserved failures

- `initial-check`: compilation failed before Rust checks because the sandbox
  made the default compiler cache read-only. The approved rerun used its own
  cache and preserved the same protection rules.
- `focused-first`: a test supplied the FIFO mknodat type/mode parameters in the
  wrong order; no tests ran. The compiler's exact signature guided correction.
- `focused-second`: one driver returned ordinarily and 17 fixture setups
  refused because this host places `security.selinux` on every scratch object.
  Explicit bounded preservation of that label was added to the contract; ACLs,
  capabilities and other attributes still refuse. `focused-third` subsequently
  passes all 18 cases, before the final source freeze.
- `clippy-first`: two unnecessary timestamp casts failed the strict gate.
  Corrected final source passed `clippy-second` and the exact-source full suite.

Preliminary logs are retained as development attempts, not exact final-source
qualification. `focused-final`, `workspace` and the passing final gates bind the
qualified runtime. Earlier success does not hide the failures above.

## Remaining ownership and gates

Only the new before-image source/helpers/tests and the one module export change
runtime code. Preparation, custody, staging, active reconstruction, core,
authority schema, generated contracts, packaging, workflows, services and UI
remain unchanged. The active reconstruction worktree was inspected read-only.
Main and the frozen Python project received no edits, installation or test run.

The caller must supply authorized retained descriptors and labels, protected
ancestors/mount/backing facts, same-UID/privileged writer exclusion, workspace
separation and complete current policy. Source checks do not freeze writers or
create a simultaneous multi-file snapshot. Metadata-only directories are not
recursive backups; absent objects, online DB consistency, inode flags and
other unsupported metadata need separate design.

Production inventory/limits, shared physical claims, quotas, service/database
snapshots, durable before-image publication/receipts, independent worker
deadlines/descendant closure, installed headroom, protected live replacement and
uncertainty/power-loss recovery remain unfinished. **BKP-001, P04 effects and
cutover stay open.** The historical 61.401-ms inventory p95 against 20 ms remains
failed. No remote host, Pi/wybie/SSH, workstation Docker or guest run occurred.

The slice estimate is 24 human hours with review at 36; P04 stays 320/480.
Measured active agent and human effort are unavailable. Tool/approval/build
waits and elapsed calendar time are not recorded as human engineering hours.
