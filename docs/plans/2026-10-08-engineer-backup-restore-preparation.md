# Engineer 1: verified private restore preparation

Implement the next RW-043 library prerequisite: consume a genuine `staging::VerifiedCatalog` and a closed trusted managed-resource selection to prepare independently verified replacement payloads and recovery inputs in an owned private directory. Return an opaque prepared owner after all checks and durability barriers. Live destinations remain unchanged; no restore job, promotion or service coordination is included.

Start `engineer/backup-restore-preparation` in a separate worktree from `2fd74209e1238c1374837ab431ef670020726dc2` after [its full CI](https://github.com/Brownster/limeos/actions/runs/37746766303) succeeds. Read [the coordinator](2026-10-08-engineer-assignments.md), [custody](../p04-backup-archive-custody.md), [verified staging](../p04-backup-verified-staging.md), [integration proof](../rewrite-evidence/p04/2026-10-08-library-handoff-integration/README.md), [RW-043](2026-10-04-rust-rewrite-roadmap.md) and [the defect register](2026-10-04-rust-rewrite-defect-register.md). The current local baseline is 370 unit/integration plus two compile-fail tests; preserve the three reviewed custody corrections.

Provisional estimate: **24 human engineering hours**, with assignment review at **36 hours (150%)**. P04 remains 320/480 hours. Record actual agent work, waits and measured human time separately. Agree the closed preparation contract against these boundaries before implementation; missing shared interfaces are concrete proposals, not permission to edit their owners.

## Owned paths and sole shared allowance

- `crates/backup-archive/src/preparation.rs` and private helpers/tests under `src/preparation/`
- `crates/backup-archive/tests/preparation.rs`
- `tests/fixtures/backup-preparation/`
- `docs/p04-backup-restore-preparation.md`
- `docs/rewrite-evidence/p04/rw043-preparation/`

The only shared source allowance is one line, **`pub mod preparation;`**, in `crates/backup-archive/src/lib.rs`, committed separately as an additive export. It enables ordinary Cargo/CI discovery of the new module's tests. Existing normal dependencies already include serde/serde_json and rustix; add no crate/dependency/version or lockfile change. Keep custody, staging, inspector/decoder, domain types, old fixtures/evidence, core/schema/jobs/claims/approvals/receipts, API/IPC/CLI, packaging/systemd/workflows and UI unchanged.

## Preparation boundary

Use the existing genuine catalog's `manifest()`, `entries()`, `CatalogEntry::metadata()` and borrowed `reader()` API. Catalog JSON or imported metadata cannot construct a prepared owner. Tests must obtain catalogs through actual admission/replay, including fresh custody recovery; never add a public fixture/provider seam. Agree any further catalog API need with the integrator.

The trusted caller selects a closed set of resource IDs and bounded regular-file/directory and numeric owner/mode policy. Archive strings cannot choose destination roots, owners, permissions or additional resources. Refuse unknown, absent, duplicate or ambiguous selection. Initial support is regular-file replacement/new-file preparation and required managed directories; unsupported types/metadata are explicit refusals. No removal of unrelated files is included.

Private copies remain owned by the collector under its private directory/file discipline. Bind requested installed UID/GID/mode from **trusted managed-target policy** as plan facts, never from archive header authority, separately from the private object's actual metadata. Do not perform privileged chown or pretend the installed metadata was applied. Applying those facts belongs to the later approved live operation.

Prepare only under a retained private root using generated exclusive names, no-follow protected descriptor operations and checked limits. Catalog-relative paths are typed plan facts, never private object filenames or unchecked destination authority. Re-read each borrowed payload, bound its size, verify its complete checksum and identity, then verify/fsync/seal the private copy. Mutation after staging must yield no usable prepared owner. Bind full selected policy, catalog/manifest identity and per-resource/per-file bytes and policy-owned metadata, with bounded canonical records and fallible allocation before growth. Preserve both primary and cleanup failures.

If existing-target recovery capture is included, consume only caller-authorized retained managed descriptors. Read and verify independent private copies and their metadata/checksums; a pathname or hard link is not a recovery snapshot. Do not open a live target for writing or broaden selection. A free-space check is not disk reservation; global quotas and authority belong to the future caller.

Every failure/cancellation returns no prepared owner. Cleanup only verified owned inodes; retain/report unknown or replaced names rather than deleting them. Published preparation records, if chosen, still require exact trusted binding and fresh validation; no automatic orphan adoption or old catalog/staging restart is allowed. Restart reconstructs a fresh genuine catalog from custody plus a future trusted job binding.

## Meaningful acceptance

Use real private scratch trees and gzip/zstd catalogs from the existing inspector/stager. Prove managed destination bytes and unrelated sentinels remain unchanged in every case.

| Scenario | Required proof |
|---|---|
| Selected regular files/directories, exact bytes, trusted installed metadata facts and boundary sizes | Complete verified collector-owned private copies with deterministic bound records/readers; no privileged ownership change |
| Changed catalog payload, policy/manifest identity, unknown/duplicate/missing resources | Whole preparation refuses, no imported owner or broadened selection |
| Traversal, symlink/magic-link escape, wrong type, inode/name replacement | No following, live write, clobber or foreign deletion |
| Quota/overflow, allocation/ENOSPC, short/interrupted/zero writes, cancellation, each fsync/rename failure | Bounded typed outcome with primary and cleanup facts preserved; ambiguity is explicit |
| SIGKILL before/after chosen durability barriers, unknown/replaced private objects | No partial authority or automatic orphan adoption; owned child is reaped and unrelated trees survive |

Record file/record/disk limits, maximum bytes/objects and descriptors, and distinguish deterministic injected faults from real scratch/SIGKILL behavior. No SSH/Pi/wybie, live VM/production operation or original Python change is authorized. SIGKILL and fsync fault tests do not prove real power-loss or installed recovery.

Run focused meaningful tests and required fmt, strict workspace/all-target Clippy, workspace tests, cargo-deny/audit, repository/contracts and diff checks. Coordinate shared Cargo caches; locked/offline checks must disclose cached advisory scope. Return exact base/tip/history, absolute worktree, allowed-path diff, API/lifecycle doc, hashes, actual counts, raw failures/passes and remaining gates on your committed feature branch. You may push only that branch; the integrator owns main and merged qualification.

**BKP-001 stays open.** Durable job/policy/approval binding, shared resource claims, snapshots/quiescing affected services, protected live replacements, uncertainty journaling, installed receipt/recovery and power-loss rehearsal remain integrator work. This library authorizes none of them.
