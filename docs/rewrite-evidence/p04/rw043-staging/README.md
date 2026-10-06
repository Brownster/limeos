# RW-043 verified staging handoff

Review branch: `engineer/backup-verified-staging`, isolated worktree
`/tmp/limeos-backup-staging`, based on `a595c2f1bcca8a8a0bc21ea0162eea841d0d5d15`.
Implementation: `8d547d9`; qualification helpers/source revision: `ffef192`.
This extends the admitted v2 archive interface; **BKP-001 stays open**.

## Scope and identity

The library captures the full initial typed policy plus inspected compressed
archive identity, reuses the inspector's streaming path, writes only flat
internally named private quarantine files, and returns borrowed readers only
after exact manifest equality, independent file verification and fsync. It
never writes into policy destination roots. See the [API/trust contract](../../../p04-backup-verified-staging.md)
and [executable example](../../../../crates/backup-archive/examples/stage.rs).

Only `backup-archive`, two domain finding variants, its existing dependency
edges, staging fixtures and new documentation/evidence changed. Dependency
versions/checksums are unchanged: `rustix 1.1.5` and `getrandom 0.2.17` were
already pinned. The domain remains free of I/O dependencies and first-party
`unsafe_code = "forbid"` remains enforced. No credentials, databases or `.env`
values occur in generated fixtures.

The original 19 integration and three inspector unit tests remain unchanged.
The source hash inventory and raw final output are in
[checks.json](logs/final/checks.json); [run_checks.py](run_checks.py) records
commands, effective UID, platform, commit and every exit code. All tests use
the workstation effective UID, not root. Compiler/uv cache access required
running outside the filesystem sandbox, without changing UID or using sudo.
No SSH, Pi, VM, installed services or Python reference writes were used.

## Local checks

| Gate | Result |
| --- | --- |
| Rust 1.88 fmt | Passed |
| Strict workspace Clippy, all targets | Passed with `-D warnings` |
| Workspace tests | **251 passed**, versus 231 at the base: 19 new substantive cases and one crash helper |
| Backup crate | 10 unit + 32 integration tests passed |
| Repository boundary/hygiene and generated contracts | Passed |
| Locked offline metadata | Passed; [source-delta.json](source-delta.json) confirms all 212 external package identities unchanged |
| Cached cargo-deny 0.20.2 | Advisories, bans, licenses and sources passed; three existing duplicate-version warnings |
| Cached cargo-audit 0.22.2 | Exit 0, 227 lockfile packages against 1,290 cached advisories; no fetch/yanked lookup. Database revision/time recorded in source delta |
| Local release usage example | Built and executed successfully in all 18 footprint runs |

Successful commands and complete raw output are in [final logs](logs/final/checks.json).
The dependency/module declarations were committed separately (`ce6441e`,
`7fae19a`). The original admission sections are byte-identical after trimming
outer whitespace; their hashes are recorded in source delta.

## Bounded memory

Three fresh release processes per case perform admission, replay, independent
catalog reads and observed discard. Input is deterministic synthetic data.
Each result includes the archive/payload digest and source/binary/policy hashes;
the example rejects a reader size/hash mismatch. The complete stdout/stderr,
exit codes, commands and cleanup observations are preserved in
[compressed raw samples](peak-memory-raw.json.gz); [summary](peak-memory.json)
includes their artifact hash. The example binary is 894,376 bytes on this host.

| Fixture | Payload/files | Peak RSS (KiB, min–max) |
| --- | --- | --- |
| Small gzip | 128 KiB / 1 | 2,952–2,992 |
| Large gzip | 64 MiB / 1 | 2,956–3,076 |
| Small zstd | 128 KiB / 1 | 3,164–3,244 |
| Large zstd | 64 MiB / 1 | 5,292–5,348 |
| High-expansion zstd zeros | 64 MiB / 1 | 5,360–5,412 |
| Metadata zstd | 0 bytes / 1,000 | 5,096–5,116 |

The gzip result stays near 3 MiB as the file grows 512-fold. The large zstd
case adds about its permitted 2 MiB decoder window, not a retained 64 MiB file.
The metadata case includes bounded report/catalog metadata, held descriptors
and the example's JSON formatting, independently of payload bytes. RSS excludes
kernel page cache and descriptor accounting. Staging still needs disk space
for all admitted file bytes and enough descriptors for the bounded file count.
These workstation samples establish streaming behavior, not installed/ARM64
latency or power-loss recovery.

## Acceptance matrix

The named integration cases below are in
[archives.rs](../../../../crates/backup-archive/tests/archives.rs); private
fault cases are in [staging/tests.rs](../../../../crates/backup-archive/src/staging/tests.rs).
All original admission cases still execute through the common streaming path.

| Requirement | Local evidence |
| --- | --- |
| Real gzip/zstd; representative and existing GNU/POSIX fixtures | `staging_replays_real_formats_and_gnu_fixtures_to_flat_private_files`: independent reader SHA/size, deterministic manifest, literal case, one flat file per original, directory metadata only, 0700/0400 modes; managed sentinel unchanged |
| Primary legacy repeats | Same case includes `legacy-primary-overlap.tar.zst`; original inert-repeat/link-adversary tests remain; manifest repeats retained, staged file count equals original file count, no links |
| Changed payload/header/order/format and caller-supplied digest | `staging_changed_payload_order_header_format_and_matching_supplied_digest_refuse`; snapshot rejects substituted digest even when sorted entry facts match |
| Full policy, including same-revision edits | `staging_refuses_full_policy_edits_even_with_the_same_revision_before_reads`: no source read or attempt creation on refusal |
| Forged version/revision/digest/counts/totals/paths/resources/checksums | `staging_revalidates_every_manifest_field_and_never_uses_forged_paths`: 21 mutations including overflow totals; foreign sentinel preserved |
| Traversal, absolute names, links, devices, sparse/FIFO, duplicate destinations, metadata overrides | Original 19 cases unchanged plus `staging_corrupt_truncated_trailing_and_hostile_streams_clean_private_attempts`; real encoders and hashed GNU hostile fixtures |
| Integrity refusal after tentative payload writes | `staging_footer_failure_and_mid_file_cancellation_follow_tentative_writes` observes nonzero private file size before CRC, truncation, trailing-data and mid-file cancellation refusal; root empty afterwards |
| Entry/file/metadata/window/bomb limits | Original exact-limit and bomb cases plus `staging_enforces_declared_metadata_file_entry_and_decoder_limits`; expected totals may reject earlier than policy-only inspection |
| Aggregate file bytes/count, checked allocation | `allocation_and_total_file_byte_quotas_fail_before_creating_files`: overflow reservation, declaration before create, second-file count quota, partial attempt drop cleanup |
| One-byte and repeated interrupted reads; near EOF cancellation | `staging_short_interrupted_reads_and_cancellation_mid_stream_and_near_eof`, `staging_repeated_interrupted_reads_preserve_bytes`; both formats |
| Partial/interrupted/zero writes; cancellation after final fsync | `partial_write_and_each_durability_failure_are_typed_and_cleaned`, `short_interrupted_zero_writes_and_cancellation_before_completion` |
| Independent verification of stored bytes | `independent_size_and_hash_verification_refuse_a_lying_or_corrupt_writer`: writer claims success after omitting/corrupting bytes; no catalog |
| File, directory, root fsync faults | `partial_write_and_each_durability_failure_are_typed_and_cleaned`: injected fsync failures at all three boundaries; typed I/O and empty root |
| Input reader failure | `input_io_failure_is_not_misclassified_as_archive_syntax`: typed I/O finding rather than malformed tar |
| Root/ancestor symlinks and pre-existing foreign entries | `staging_root_symlink_and_foreign_path_attacks_refuse_without_overwrite`, `existing_attempt_and_file_symlink_attacks_never_overwrite_or_delete_foreign_files`; no overwrite/follow/recurse |
| Held descriptor cleanup and independent owned attempts | `staging_attempts_are_independent_and_cleanup_uses_held_root_descriptor`, `cleanup_refuses_a_replaced_attempt_name_and_keeps_foreign_directory`; root rename cannot redirect cleanup |
| Real process interruption and no automatic resume | `staging_sigkill_leaves_private_incomplete_artifacts_and_never_resumes_them` kills a dedicated synthetic child after nonzero tentative writes, verifies SIGKILL/0700/0600, then replays a separate attempt without touching the orphan |
| Small versus 64 MiB data and separate metadata cost | [measure_staging.py](measure_staging.py), synthetic [fixture policy](../../../../tests/fixtures/backup-staging/policy.json), release `stage` example; three fresh processes per case |

`staging_crash_child` is a helper test with no work unless the parent supplies
its private root through a dedicated environment variable. Count it separately
from substantive regressions when interpreting the workspace test total.

## Retained failures and corrections

[Initial compile output](logs/check-initial.txt) records an ambiguous numeric
counter type, corrected to `u64`. [First tests](logs/tests-first.txt) record
test roots created as 0755 by tempfile/umask: the library correctly refused
them. Explicit 0700 fixture construction fixed that setup; the refusal test
remains. [Second tests](logs/tests-private-roots.txt) exposed a real identity
gap: sorted entries permitted changed tar order when only a policy snapshot
was trusted and the caller substituted the compressed digest. Initial admission
now binds that digest inside the private snapshot.
[Binding regression output](logs/tests-admission-binding.txt) shows the corrected
case passing. Strict Clippy also caught the example's domain rejection/error
conversion, a complex test type and unnecessary drops of borrowed sinks;
these were corrected without weakening lint settings.

## Remaining integration

Disk attempts always retain an `incomplete-*` name, even after library success.
There is no on-disk success catalog, resume or promotion API. Drop cleanup is
best effort; explicit `discard` reports cleanup failure. Foreign replacement
or an obstructing entry is retained and reported, not recursively deleted.
SIGKILL evidence is not power-loss or installed recovery evidence.

The later executor must enforce archive/staging immutability, human approval,
resource claims and locks, protected destination traversal, policy-owned
ownership/permissions, recovery snapshots/receipts, service coordination,
atomic replacement and independent post-restore verification. Production
limits/mappings come from P00. Integrator review/merge remains sequential;
this source needs its own installed/native qualification after integration.

## Effort

Recorded agent wall interval: 2026-10-06 21:38:05–22:49:19 UTC, about **1.2 hours**,
including local compilation, tests and measurement waits. Brief familiarization
before the first timestamp was not separately metered; this is an agent session
measurement, not a human engineering-hour claim. The assignment's 24-hour
estimate and 36-hour remaining-scope review remain unchanged; no threshold was
reached. P04's 320-hour estimate and 480-hour cutover-scope review remain project
budgets. Installed executor/recovery qualification is remaining integration work.
