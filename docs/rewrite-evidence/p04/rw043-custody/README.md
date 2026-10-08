# RW-043 private archive custody: handoff

| Item | Value |
|---|---|
| Branch | `engineer/backup-catalog-recovery` |
| Worktree | `/home/marc/Documents/github/lime-os-backup-catalog-recovery` |
| Base | `d324c1b9f3ee8dcc46d7d5deb4cf00c25b1301f4` (runtime and fixtures `e330065`) |
| Source of all checks below | `9a8232262d8de552e10bd88126395cfe197ee634` |
| Brief | [Engineer 1: private archive custody and restart staging](../../../plans/2026-10-07-engineer-backup-catalog-recovery.md), added on main in docs-only `b742f24` after this base; the link resolves once merged |
| Contract | [Archive custody and restart staging](../../../p04-backup-archive-custody.md) |

**BKP-001 and the P04 effect and cutover gates stay open.** This library retains an admitted
compressed archive, recovers it by an exact trusted binding after restart and
stages a fresh catalog. It restores nothing.

## Commits

| Commit | Content |
|---|---|
| `5d811f4` | Shared-file allowances only: `pub mod custody;`, the pinned `serde` edge, `serde_json` moved from dev to normal; a one-line module stub so the commit builds |
| `9a55efc` | Custody implementation, identity encoding, private fault and SIGKILL tests |
| `7a9866a` | Public-API integration tests |
| `9340277` | The fault seam calls the production no-replace rename (so a weakened rename is observable) |
| `72caf4c` | Usage example and synthetic fixtures |
| `9cfb67b` | API and lifecycle contract |
| `9a82322` | Qualification helpers: check runner, mutation checks, measurement, Python identity reference |
| (this commit) | Evidence: raw logs, measurement output and this record |

## Effort

| Item | Value |
|---|---|
| Estimate | 24 engineering hours; review point 36 hours |
| Agent time | One session on 2026-10-07, from 19:32Z to about 20:20Z. That is elapsed agent time, not human engineering hours, and doesn't revise the estimate. |
| Infrastructure waits | Required checks 2 min 54 s; two mutation runs of 46 s each; release measurement about 1.5 min; earlier focused test runs of about 20 s each. No approval waits. |
| Human time | None measured |

## Required checks

[run_checks.py](run_checks.py) ran every command from the repository root at
`9a82322`, as the workstation user (UID 1000, x86-64, Linux 7.0), offline with
locked dependencies. Raw output is in [logs/final](logs/final/), and the
commands, exit codes, elapsed times, log hashes and source hashes are in
[checks.json](logs/final/checks.json).

| Check | Result |
|---|---|
| Focused crate tests (`cargo test -p limeos-backup-archive --locked`) | Pass: 20 unit (10 existing + 10 new), 32 existing integration, 11 new custody integration |
| `cargo fmt --all -- --check` | Pass |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | Pass |
| `cargo test --workspace --locked` | **312 passed, 0 failed, 0 ignored.** Base 291 + 21 new: 20 substantive tests and the `crash_child` subprocess helper, which does nothing unless the parent sets its environment. |
| `cargo deny check` (0.20.2, offline) | `advisories ok, bans ok, licenses ok, sources ok`. Three duplicate-version warnings (`base64`, `getrandom`, `syn`) predate this branch. |
| `cargo audit` (0.22.2, `--no-fetch`) | No vulnerabilities in 227 lockfile crates against 1,290 advisories. **Cached** advisory database `ef6173cb`, dated 2026-10-03; no fetch or yanked lookup. |
| `python3 scripts/check_repository.py` | Pass |
| `python3 scripts/check_contracts.py` | Pass; generated contracts unchanged |
| `git diff --check d324c1b..HEAD` | Pass |
| Release example build | Pass; 1,045,944 bytes |

### Ownership and dependency review

The same run's [ownership review](logs/final/checks.json) passed:
- **Changed paths:** all are owned paths or the allowed shared edits.
- **`lib.rs`:** gains exactly `+pub mod custody;`.
- **`Cargo.toml`:** adds `serde.workspace = true` and moves `serde_json.workspace = true` from dev to normal dependencies.
- **Lockfile:** all 227 packages and all 211 external entries keep their version, source and checksum. The only lockfile change is `serde` added to `limeos-backup-archive`'s dependency list, a direct consequence of the allowed edge.
- **Protected paths:** byte-identical to the base, including `staging.rs`, staging tests, `archives.rs`, the inspector, domain types, archive and staging fixtures, old evidence and generated contracts.

There are no other workspace or manifest edits, and no new crate or version.

## Case-to-test matrix

Integration tests are in [tests/custody.rs](../../../../crates/backup-archive/tests/custody.rs); private seam tests are in [src/custody/tests.rs](../../../../crates/backup-archive/src/custody/tests.rs). The Kind column says what each case uses:
- **Real:** real files in private scratch directories.
- **Injected:** a private deterministic fault seam over real files.
- **SIGKILL:** a real child process killed after it reports a checkpoint on stdout.

| Scenario | Tests | Kind |
|---|---|---|
| Small and 64 MiB archives in both codecs, with content-based detection under a misleading `.tar.bz2` name; retain, close, exact-binding recover, fresh stage, independent reader hashes, discard; repeated recovery; catalog Drop | `custody_small_archives_retain_recover_stage_and_discard_in_both_formats`, `custody_64_mib_compressed_and_expanded_archives_complete_the_lifecycle` (64 MiB incompressible stored gzip and 64 MiB of zeros in zstd) | Real |
| Current self-repeat fixture and current decisions | `custody_preserves_current_fixture_decisions_and_the_inert_self_repeat`. `legacy-primary-overlap.tar.zst` keeps 2 coalesced repeats, and staging creates one flat single-link file per regular file. The three valid GNU fixtures complete the lifecycle. Symlink, FIFO, sparse, absolute, parent and 64 MiB-declaration fixtures are refused with their original findings and leave no custody state. | Real |
| Same-revision policy edits and a revision change | `custody_full_policy_changes_refuse_recovery_and_staging_even_with_the_same_revision`. 19 edits: format removed or reordered, each of the 10 limits, registry root, resource order, mapping order, prefix, target, an extra mapping and the revision. Each refuses both recovery and staging, with no staging attempt. | Real |
| Binding tampering | `custody_binding_substitution_is_refused_by_exact_binding_validation`. 11 cases: unknown, uppercase, short and path-like IDs; version; zero size; archive digest; size; policy and manifest identities; over-limit size. Unknown and duplicate binding JSON fields are rejected. | Real |
| Forged, malformed, oversized and foreign records | `custody_forged_malformed_oversized_and_foreign_records_are_refused`. 9 malformed encodings: unknown top-level and nested fields, a duplicate field, trailing data, trailing whitespace, truncation, a pretty-printed non-canonical form, a wrong version, and non-JSON. 7 canonical but inconsistent forgeries: an entry removed, file count, an entry checksum, saved policy root and order, record ID, and archive digest. Also a forged `complete` success marker, an over-limit record and a record shrunk below the limit. Restoring the exact bytes restores recovery. | Real, same-UID tampering |
| Source pathname replacement; non-regular sources | `custody_source_replacement_cannot_redirect_the_selected_descriptor`: the pathname is replaced after selection and the held descriptor's bytes are retained; a directory and `/dev/null` are refused | Real |
| Sealed bytes changed by append, truncation, same-size reordering, same-size corruption; inode replacement and in-place edits under a held owner | `custody_sealed_byte_size_order_and_inode_changes_refuse`. The reordered archive has equal sorted entries but a different compressed identity and is refused as `BindingMismatch`. | Real |
| A source that grows or shrinks after selection | `changing_source_is_refused_after_reading_only_the_sentinel`: exactly limit + 1 bytes are read before refusal; a shrunk copy is judged as copied | Injected |
| Root attacks: relative path, symlink, symlinked component, `/proc/self/fd` magic link, mode 0750, regular file, absent; root renamed while held. Object attacks: archive, record and record-directory symlinks to valid copies, hard link, wrong modes, a directory in place of the record, foreign entries. | `custody_root_and_object_substitution_never_follows_clobbers_or_deletes`. Nothing is followed, and the symlink targets and foreign entries survive. | Real |
| Generated-name collision (pending, published, and published appearing just before the rename); rename `EIO` and `EINVAL` (no `RENAME_NOREPLACE`) | `rename_failures_and_generated_name_collisions_never_replace_foreign_entries` | Injected fixed ID and faults; real `renameat2` |
| Exact copy limit; one byte under; invalid limits and policy; immediate cancellation | `custody_limits_and_policy_are_validated_before_any_custody_state` | Real |
| Short and interrupted reads and writes | `short_and_interrupted_reads_and_writes_preserve_the_exact_copy` | Injected |
| Zero writes; `ENOSPC` part-way through the archive and the record | `zero_failed_and_full_writes_are_typed_and_cleaned` | Injected |
| Every fsync barrier: archive, record, pending directory, root after rename (ambiguous durability, then exact-binding recovery), root after discard | `every_durability_barrier_failure_is_typed_and_publication_is_never_rolled_back` | Injected |
| Cancellation at each pre-publication point; ignored after rename; during re-admission and replay | `cancellation_rolls_back_only_before_publication` | Injected |
| Cleanup that finds a replaced owned name or a foreign entry | `pending_cleanup_reports_replaced_and_foreign_objects_and_keeps_them`: primary and cleanup errors are both reported, and foreign objects are kept | Injected |
| SIGKILL during copy; after archive fsync; after admission; after record and pending fsync; after rename before root fsync; after all barriers before return; during restart re-admission; during replay | `sigkill_at_every_custody_point_leaves_untrusted_or_freshly_validated_state`, with helper `crash_child`. Pre-publication: a 0700 `pending-*` orphan, `NotFound` even for a correct binding, and a later retention that leaves it untouched. Post-rename: recoverable only with the exact binding after full validation; a wrong identity is refused. Re-admission and replay: custody is byte-identical; a replay leaves an `incomplete-*` orphan that a fresh stage neither adopts nor removes. The unrelated sentinel and the source survive every case. | SIGKILL |
| Retained Drop, explicit discard, independent owners, repeated recovery, fresh catalog Drop, discard of a replaced record | `custody_owners_are_independent_and_drop_never_removes_published_state` | Real |
| Identity encoding: known answers, order sensitivity, length prefixes, exact bound | `identity_encoding_is_versioned_order_sensitive_and_bounded` | Pure |
| Scope and unchanged regression suites | Ownership review above; the 32 existing integration and 10 existing unit tests pass unchanged; [static scope scan](logs/scope-scan.txt) | Static and real |

### Mutation checks

[mutation_checks.py](mutation_checks.py) weakened one guarantee at a time at
`9a82322` and ran the custody unit and integration tests, skipping only the
64 MiB case ([summary](logs/mutations/mutations.json), [raw logs](logs/mutations/)).
**14 of 14 targeted mutants were caught** by named tests:
- full-policy checks in recovery and in staging
- canonical record bytes and record-binding equality
- foreign entries and single-link objects
- the copy sentinel limit
- hiding ambiguous durability
- sorted-entry-only re-admission
- held-identity checks
- inode-checked discard and pending cleanup
- the no-replace rename
- identity length prefixes

Two mutants survive, as predicted: removing either the full policy equality
or the policy identity comparison on its own. Each check independently
refuses a policy change, so neither is the only guard. An earlier run at
`9340277` ([logs](logs/mutations-9340277/)) had identical outcomes; the custody
source didn't change between the two commits.

### Independent identity reference

[identity_reference.py](identity_reference.py) implements the version 1 encoding
from the contract's text, not from the Rust code. For the unit-test policy
([logs/unit-test-policy.json](logs/unit-test-policy.json)) it computes
`5ab96685…`, 304 encoded bytes. Those are the Rust known-answer value and the
exact bound asserted in the unit test. It also recomputes both identities for
four records written by the release example from the GNU fixtures (long names,
directories, pax and coalesced repeats), and all four match their bindings
([output](logs/identity-reference.txt)).

## Release measurement (optional)

[measure_custody.py](measure_custody.py) ran the release example three times
per case. Each run used fresh private roots and two fresh processes, as a
restart would: `retain`, then `recover … --discard`. Recover verifies,
re-admits, replays, hashes every reader independently and discards both the
catalog and the record. Inputs are deterministic. Summary: [peak-memory.json](peak-memory.json).
Complete stdout, stderr, commands and digests are in [raw output](peak-memory-raw.json.gz).

| Case | Compressed bytes | Payload bytes | Record bytes | Peak RSS, retain (KiB) | Peak RSS, recover and stage (KiB) | Retain (s) | Re-admit (s) | Replay (s) |
|---|---|---|---|---|---|---|---|---|
| Small gzip | 131,303 | 131,072 | 1,860 | 2,988–3,088 | 2,948–3,096 | 0.003 | 0.002 | 0.002 |
| Small zstd | 131,613 | 131,072 | 1,860 | 3,048–3,184 | 2,928–3,184 | 0.003 | 0.002 | 0.002–0.003 |
| 64 MiB gzip | 67,129,467 | 67,108,864 | 1,870 | 2,956–3,000 | 2,744–3,032 | 1.23–1.36 | 0.59–0.62 | 0.93–1.02 |
| 64 MiB zstd | 67,110,941 | 67,108,864 | 1,870 | 5,104–5,424 | 5,188–5,440 | 1.21–1.27 | 0.59–0.62 | 0.92–1.02 |
| `zeros-64m.tar.zst` fixture | 2,214 | 67,108,864 | 1,992 | 5,096–5,404 | 5,320–5,428 | 0.29–0.32 | 0.29–0.35 | 0.64–0.67 |

Memory stays flat as the archive grows 512-fold. It is about 3 MiB, plus the
2 MiB zstd window when zstd is used. These figures match the staging-only
release measurements. Retention costs one copy, one rehash of the sealed copy
and one admission pass. Recovery costs one more admission pass, and staging
one replay pass with fsyncs.

Peak disk use beyond the source is the compressed archive plus the record plus
the payload, while a catalog exists. That is about 128 MiB for the
incompressible 64 MiB case and 64 MiB for the zeros fixture, almost all of it
in staging.

These are x86-64 workstation figures with a warm page cache. They make no Pi,
ARM64, service-footprint or installed-restore claim. RSS excludes the kernel
page cache.

## Retained failures and corrections

Every pre-commit failure is described in [attempts.md](logs/attempts.md):
- two Clippy `type_complexity` errors in the new tests
- the intentional known-answer placeholders failing before the independent cross-check
- an evidence pipeline that briefly recorded `sed`'s exit status in place of the reference's

## Not tested

- **Power loss:** SIGKILL and injected fsync faults don't prove power-loss durability.
- **Real ENOSPC:** no full filesystem was used; ENOSPC was injected.
- **Owner checks:** they are implemented, but not executed with foreign-owned objects, which needs privileges. Mode, type and link checks are executed.
- **Concurrent writers:** concurrent same-UID or privileged writers during a held operation are outside the caller's exclusion contract and are not prevented.
- **Platforms:** no native ARM64, Pi, VM, installed service, restore executor, job record or receipt integration. No SSH or wybie access was used.
- **Production values:** P00 production limits and mappings remain open; the fixtures are synthetic.

## Remaining gates

Still with the integrator:
- trusted production target and import policy
- installed job and approval binding, including durable binding delivery
- orphan, retention and quota policy
- complete shared claims and locks
- live destination, filesystem and mount qualification
- trusted owners and modes
- snapshots and affected-service coordination
- fsynced destination replacement
- post-effect receipt reconciliation

The library closes none of them. The historical storage-inventory result,
61.401 ms against 20 ms, remains failed. [SHA256SUMS](SHA256SUMS) covers this
directory's files and excludes itself.
