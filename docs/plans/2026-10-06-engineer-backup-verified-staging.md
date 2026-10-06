# Engineer 2: verified archive replay and private staging

Extend the integrated RW-043 inspector so a future restore executor can obtain exactly the inspected file bytes in private staging. Deliver the library and adversarial tests while the integrator owns RW-040 runtime effects and engineer 1 owns overnight native qualification. This assignment needs no Pi access.

Start `engineer/backup-verified-staging` in a separate worktree from `a595c2f1bcca8a8a0bc21ea0162eea841d0d5d15`. The integrated workspace passes 231 tests. Read the [archive interface and future executor rules](../p04-backup-archive-admission.md), [integration evidence](../rewrite-evidence/p04/2026-10-06-handoff-integration/README.md), [original admission brief](2026-10-06-engineer-backup-archive-admission.md), [BKP-001](2026-10-04-rust-rewrite-defect-register.md) and [architecture](2026-10-04-rust-rewrite-architecture.md).

Estimate: 24 engineering hours for this bounded library/staging slice; record actual effort. Review remaining scope at 36 hours. This remains part of P04's 320-hour estimate and 480-hour cutover-scope review threshold.

## Required interface and invariants

1. Supply a replay/staging interface taking a bounded archive reader, current trusted `AdmissionPolicy`, the expected admitted manifest and the trusted policy identity captured at initial admission. Reuse the inspector's actual decoding, effective-name handling, limits and EOF verification. Refactor a common internal streaming path if needed; do not add a second tar parser or invoke system tar for extraction.
2. Revalidate the expected manifest's version and policy revision and verify all recomputed facts against it: full compressed digest, format, byte/header counts, file/directory counts, inert-repeat count and every mapped entry/checksum. Bind the initial policy's full content through an immutable typed snapshot or documented canonical fingerprint carried alongside the v2 manifest; revision equality alone cannot detect same-revision policy edits. A changed archive, changed policy or forged/stale manifest must never yield a completed staging result. A matching caller-supplied digest is not sufficient proof.
3. Stream file data into an attempt-specific private quarantine using bounded buffers. Archive paths and link targets never name files on the host, even inside quarantine. Generate staging names internally; bind file handles to managed resource ID/relative path only in the catalog. Preserve literal path/case in that metadata. Directories produce directory metadata, not arbitrary host directory trees.
4. Return a completed catalog only after the entire replay has passed integrity and manifest equality checks, and staged files have passed their own size/checksum and required durability checks. File chunks emitted before EOF are tentative, not verified outputs. Prefer an API that owns quarantine until successful completion; an arbitrary callback that can publish bytes early must not be presented as safe restore execution.
5. Use private directory/file permissions and protected descriptor operations. No symlink-following opens, overwritten pre-existing files, archived ownership, special bits, xattrs or ACLs. Reuse already-pinned rustix/tempfile support where suitable; preserve first-party `unsafe_code = "forbid"`. Document the caller's trusted staging-root requirements and the later executor's immutable-source requirement.
6. Bound total staged bytes, file/entry count, metadata, compressed/decompressed reads and decoder window by trusted limits and expected manifest totals. Honor cancellation and preserve short/interrupted-read behavior. Check oversized declarations before reads/allocations. Surface I/O, allocation/quota and fsync failures as typed failures; never return a partial catalog as completed.
7. On normal error/cancellation/drop, remove only this attempt's private files. A killed process may leave private incomplete artifacts; distinguish them explicitly and never promote or resume them automatically. Describe crash cleanup and later receipt integration honestly. No success marker/catalog may become visible before verified completion.

The completed catalog is evidence, not restore authority. It may expose protected file handles plus immutable catalog metadata to the future executor, but cannot copy into live managed destinations. Fresh human approval, resource claims, protected destination traversal, policy-owned ownership, recovery snapshots, receipts, service coordination, atomic replacement and independent post-restore verification remain with later integration.

## Legacy compatibility

Keep manifest version 2 and the reviewed inert-repeat rule: only a zero-length hard link targeting its exact own path after a fully inspected regular file at that path is coalesced. It produces one staged file, no link and no additional destination; the manifest retains the repeat count and full archive binding. Other hard links, symlinks, forward references, sparse/device/FIFO entries and path aliases remain rejected. The first file still must pass trusted mappings and all destination checks.

Production limits/mappings still come from the P00 inventory. Synthetic test values are not production defaults. Credentials and databases must not acquire a new restore policy in this slice. Do not add encryption, online database snapshots, schedules, HTTP routes or automatic service stop/apply behavior.

## Acceptance matrix

Exercise real gzip and zstd replay against private temporary directories:

| Case | Required evidence |
|---|---|
| Valid representative and GNU fixtures | Staged bytes match manifest checksums and sizes; deterministic catalog mapping; no writes to sentinel managed destinations |
| Primary legacy self-repeats | One staged file per original file, repeat count retained, no link creation; unsafe link variants rejected |
| Changed payload/header/order/count/format/digest or policy revision | Manifest mismatch/refusal; no completed catalog |
| Forged relative paths/resources/checksums/totals or unsupported manifest version | Refusal without publishing files or deriving filesystem names from the forged values |
| Traversal, absolute paths, link escapes, devices, sparse files, duplicates/collisions, metadata overrides | Original inspector decisions preserved through the same streaming path |
| Corrupt footer, truncation or trailing data after valid file bytes | No completed catalog despite having tentatively written data; owned quarantine cleaned on normal failure |
| Entry/byte/metadata/window limits and bombs | Bounded rejection; aggregate staging quota cannot be bypassed by repeats or multiple files |
| One-byte reads and interruption/cancellation mid-file or near EOF | Same admitted bytes on valid input; cancellation yields no completed result |
| Writer/fsync failure and private-root symlink/pre-existing file attacks | Typed failure; no overwrite, escaped write, or deletion of foreign sentinels |
| Dropped/abandoned attempt and process interruption | Cleanup where possible; surviving artifacts stay private/incomplete and cannot be auto-accepted |
| Small versus 64 MiB streamed payload | Evidence that file data is not retained as a whole in RAM; distinguish buffer cost from bounded catalog metadata |

Use deterministic, bounded synthetic fixtures and injected I/O faults. Record source/fixture hashes and raw successful/failed output. Never commit real credentials, database contents or `.env` values.

## Ownership and handoff

Own `crates/backup-archive/`, narrow additions to `crates/domain/src/backups.rs` and its tests if needed, `tests/fixtures/backup-staging/`, `docs/p04-backup-verified-staging.md` and new evidence under `docs/rewrite-evidence/p04/rw043-staging/`. Keep unrelated domain storage/pool/protection modules, core, persistence, contracts, executor protocol/services, packaging and frontend unchanged. Module exports or dependency declarations belong in separate small commits; prefer existing pins, and document any graph change. Do not weaken admission tests or overwrite the original engineer/integration evidence.

The other engineer qualifies frozen source `8acac40`; your later source must have its own test count and proof. Local staging tests run unprivileged on the workstation. Do not use wybie or share the Pi window. The frozen Python checkout remains read-only.

Deliver a reviewable branch with fmt, strict Clippy, workspace tests, repository/contract and dependency checks, a small API usage example, the malicious-input/I/O matrix, bounded-memory evidence and the remaining executor requirements. **BKP-001 stays open** until the installed restore operation consumes this path and proves recovery. The integrator reviews and merges shared runtime changes sequentially.
