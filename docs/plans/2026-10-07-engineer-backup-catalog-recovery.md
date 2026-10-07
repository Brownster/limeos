# Engineer 1: private archive custody and restart staging

Implement the next independent RW-043 library prerequisite. Retain an admitted compressed archive with a trusted identity binding, revalidate and re-admit it after restart, then generate a **fresh** `VerifiedCatalog` through the existing staging API. Existing catalogs stay ephemeral; this assignment does not resume old staging directories or restore live destinations.

Start `engineer/backup-catalog-recovery` in a separate worktree from `d324c1b9f3ee8dcc46d7d5deb4cf00c25b1301f4`. Read [the coordinator](2026-10-07-engineer-assignments.md), [archive admission](../p04-backup-archive-admission.md), [verified staging](../p04-backup-verified-staging.md), [RW-043 and the phase gates](2026-10-04-rust-rewrite-roadmap.md), [the defect register](2026-10-04-rust-rewrite-defect-register.md) and [current qualified source](../rewrite-evidence/p04/2026-10-07-handoff-integration/README.md). Its runtime/fixtures are `e330065`: 291 Rust tests, 33 ARM harness tests per native architecture, 9 separate reference tests and 180 installed AMD64 groups.

Provisional human estimate: **24 engineering hours**, with assignment-scope review at **36 hours (150%)**. Record agent time, infrastructure waits and measured human time separately. P04 remains 320 hours with a distinct 480-hour cutover-scope review. Narrow the task if needed; do not expand into persistent payload catalogs, automatic orphan adoption or destination effects.

## Exclusive ownership

- `crates/backup-archive/src/custody.rs` and optional private helpers under `crates/backup-archive/src/custody/`
- `crates/backup-archive/tests/custody.rs`
- `crates/backup-archive/examples/custody.rs`
- New reproducible synthetic fixtures under `tests/fixtures/backup-custody/`
- `docs/p04-backup-archive-custody.md`
- `docs/rewrite-evidence/p04/rw043-custody/`

Two shared-file exceptions: add only `pub mod custody;` in `crates/backup-archive/src/lib.rs`; if typed durable JSON is chosen, add the already pinned workspace `serde` edge and move/use the existing pinned `serde_json` edge as a normal dependency in `crates/backup-archive/Cargo.toml`. Keep these small allowances in a separate additive commit. No new crate, external crate/version, workspace dependency edit or lockfile churn is included.

Keep the inspector/decoder, `staging.rs`, existing staging tests, domain types, old archive fixtures/generator, old evidence and generated contracts unchanged. Core, IPC/protocol, claims/approvals/receipts, persistence/schema, executor services/runtime, storage, API/UI, packaging and CI orchestration stay with the integrator. Read existing fixtures without rewriting them. Return any further shared-interface need as a concrete proposal before changing its ownership scope. The other engineer owns running-process evidence; the integrator continues RW-040 outside your crate.

## Interface and custody boundary

Provide a private-descriptor custody root, an opaque retained-archive owner and a serializable binding. Idiomatic names/signatures are flexible, but the semantic API must support:

```rust,ignore
CustodyRoot::open(existing_absolute_path) -> Result<CustodyRoot, CustodyError>;
retain(selected_regular_file, trusted_policy, custody_root, limits, cancelled)
    -> Result<RetainedArchive, CustodyError>;
RetainedArchive::binding(&self) -> &CustodyBinding;
RetainedArchive::manifest(&self) -> &RestoreManifest;
recover(custody_root, trusted_expected_binding, current_trusted_policy,
        limits, cancelled) -> Result<RetainedArchive, CustodyError>;
RetainedArchive::stage(&mut self, current_trusted_policy, staging_root, cancelled)
    -> Result<VerifiedCatalog, CustodyError>;
RetainedArchive::discard(self) -> Result<(), CustodyError>;
```

`RetainedArchive` has private fields, no constructor from saved metadata/arbitrary readers and no writable descriptor exposure. Its successful Drop closes handles but preserves the published archive for restart; explicit discard reports cleanup failures. Do not change existing `VerifiedCatalog` ownership or cleanup.

The binding includes record format version, internally generated record ID, compressed SHA-256/size, **full policy identity** and complete manifest identity. Define a bounded, versioned deterministic identity encoding, preserving policy vector order. Store the full typed policy and manifest in closed bounded metadata. Reject unknown/duplicate fields, wrong versions, truncation/trailing data, inconsistent identities and oversized records. Policy revision alone is insufficient; compare complete policy values as well as hashes. Metadata or a persisted success marker never authorizes recovery. The expected binding comes from a trusted caller's future durable job record, not a discovery scan.

1. Open an existing absolute private custody root with the current descriptor/ownership discipline: `openat2`, no symlink/magic-link components, effective-UID ownership and exact private directory mode. Missing kernel protection refuses. Use retained directory descriptors for all subsequent actions. The caller remains responsible for protected ancestors, ACLs, mount identity, space reservation and exclusion of same-UID/privileged writers. Read-only handles and chmod do not prove filesystem immutability; do not add a chattr scheme.
2. Consume a selected regular-file descriptor, never a source pathname. Copy compressed bytes once into an exclusively created internally named private file, with checked limits, bounded buffers, cancellation and only a bounded overflow sentinel. Source-path replacement cannot redirect that descriptor. The completed copied bytes define the archive to approve; timestamps do not prove absence of concurrent source writers.
3. Verify copied size/digest, regular identity, single link, ownership and mode; seal read-only, close all library writable handles and fsync. Reopen through the private directory with no-follow/identity checks, then call existing `staging::admit_for_staging` on this sealed copy. Only complete verified EOF admission creates its manifest and policy snapshot. Never inspect the source and copy afterward.
4. Publish an **admitted retained archive**, not a staged catalog or authorized restore. Use a generated pending directory; fsync bounded metadata and its directory, no-replace rename under the retained root, then fsync the parent before success. Refuse collisions. If rename succeeded but final parent fsync failed, report explicit ambiguous durability rather than success or a fictitious clean rollback.
5. Recover only the exact internally named record requested by the trusted binding. Verify object names/type, owner/mode, links, sizes, inode identities and absence of foreign entries through held descriptors. Compare saved full policy with the caller's current trusted policy, including formats, limits, registry roots, mappings and order. Re-admit actual sealed bytes through the existing API, recompute compressed identity and the whole manifest, and compare both with saved facts and the trusted binding. Never deserialize a valid `PolicySnapshot` owner.
6. Explicit staging rechecks full policy and uses existing `staging::replay` with the held admitted archive, snapshot and manifest. That parser owns payload writes, checksums and fsync. Recovery alone exposes no payload readers. No system tar, second decoder, `unpack`, writer callback or live destination path is included. Archive member names never become custody filenames.
7. Typed cancellation/I/O/cleanup results preserve primary and cleanup failures. Clean only owned known names with descriptor-relative inode checks; leave foreign replacements/unknown objects intact and report them. Never recurse through foreign trees or adopt old `incomplete-*` attempts. Published data still needs exact-binding fresh validation; a crash before binding delivery leaves an orphan, not a runnable job. Automatic orphan recovery, retention cleanup and queue/receipt reconciliation stay with the integrator.
8. Use finite explicit trusted custody metadata/copy limits; admission limits remain unchanged. Describe compressed custody plus metadata plus a fresh staging attempt in disk accounting. Global retained quotas/reservations belong to the caller. ENOSPC fails safely; a free-space check is not a reservation. Synthetic limits are not production defaults.

Preserve every current admission decision. The reviewed zero-size hardlink repeat to its exact effective path after that regular file was completely verified remains accepted and inert; it creates no link. Other links, aliases, forward references, traversal and collisions remain refused. The old blanket-primary-backup rejection report is historical. Structural compatibility grants no credential/database import authority. P00 production mappings and limits remain open.

## Acceptance matrix

Use unprivileged private scratch files/directories, both gzip and zstd, and private deterministic fault seams. Every refusal proves that no catalog escaped and no outside path changed.

| Scenario | Required result |
|---|---|
| Small and 64 MiB synthetic archives; content-based codec detection; current self-repeat fixture | Retain, close, exact-binding recover, fresh stage, independently verify all reader hashes, discard; current decisions preserved |
| Each policy format/limit/mapping/root/order changes with the same revision; revision change | Recovery and subsequent staging refuse |
| Binding/record ID, archive SHA/size, manifest entries/count/checksums and policy identity tampering; forged success; malformed/oversized/foreign record | Exact trusted binding and actual-byte/full-policy validation refuse substitution |
| Source pathname replacement; sealed inode/name/byte changes, append/truncate or same-size reordered archive | Held source cannot redirect; recovery/staging require exact compressed identity, not only a sorted manifest |
| Root/components/objects replaced by symlink/magic link/hardlink, wrong type/owner/mode, foreign entries or generated-name collision | No following, clobber or foreign deletion; retained-root behavior remains descriptor-bound |
| Boundary/overflow, short/interrupted/zero writes, cancellation, ENOSPC and every file/directory/parent fsync failure | Bounded refusal, observable cleanup, explicit post-publication ambiguity |
| SIGKILL during copy; after archive fsync/before admission; after admission/before record fsync; after record fsync/before publication; after rename/before parent fsync; after all barriers/before return; during restart admission/replay | Deterministic child synchronization; partial state never trusted, unrelated files survive, published state needs trusted binding and fresh validation |
| Retained Drop, explicit discard, repeated recovery and fresh catalog Drop | Archive survives successful handle Drop; discard checks identity; catalog keeps existing ephemeral cleanup; no host effect/job completion |
| Scope and unchanged regression suites | No live destination opens, extraction process, archived owner/mode application, DB/credential import or shared core changes; existing admission/staging tests pass unchanged |

SIGKILL and injected fsync faults do not prove real power-loss durability. A release-mode small/64 MiB RSS comparison is optional: report compressed/payload sizes, disk use and reinspection/replay cost without a Pi or service-footprint claim. No SSH, Pi/wybie, expired-window reuse, production workload/package changes or original Python edits are authorized. No live VM or network run is needed for this library.

## Required checks and handoff

Run focused crate tests, then `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `cargo test --workspace --locked`, `cargo deny check`, `cargo audit`, `python3 scripts/check_repository.py`, `python3 scripts/check_contracts.py` and `git diff --check`. Coordinate full workspace runs to avoid Cargo locks. Use locked/offline caches where needed and label cached advisory scope/environment failures accurately. New tests must run under ordinary Cargo/CI discovery. Repository Python checks do not authorize changes to the frozen Python project.

Return a clean committed feature branch, exact base/tip/commit list and absolute worktree path, allowed-path diff, concise API/lifecycle document, source/fixture/output hashes, case-to-test matrix, actual counts, raw required-check logs and preserved failed attempts. Distinguish real scratch/SIGKILL results from injected faults and untested power-loss/installed scenarios. You may push **only your assigned feature branch** to `Brownster/limeos`; do not merge or push main. The integrator owns review, main publication and full exact-source merged CI/installed qualification. Never replace historical proof.

**BKP-001 and P04 effect/cutover gates remain open.** Trusted production target/import policy, installed job/approval binding, complete shared claims/locks, live destination/FS/mount qualification, trusted owners/modes, snapshots, affected-service coordination, fsynced destination replacement and post-effect receipt reconciliation stay with the integrator. This library closes none of them. The historical 61.401 ms/20 ms storage-inventory failure is unchanged.
