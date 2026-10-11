# Durable private backup preparation binding

This P04 increment persists a closed, approved private-preparation binding and recovers the exact queued job after a lost response or process restart. Schema 9 validates its complete resources and canonical approval/job links. The archive library retains authority over actual bytes and opaque private owners. The [API contract](../../../p04-backup-durable-preparation-binding.md) describes the eight authenticated Store methods and remaining trust inputs.

The source starts at `e57a0d2a909baac11c0fbb93da8b6a0e2344fe67`. The [tested snapshot](local/source-snapshot.json), SHA-256 `3f5de99ee5461d06efe962a96e137e45a3683deed6d54b7765f9e1f02369a007`, binds 25 source files. The [final snapshot](local/source-final.json), SHA-256 `d1e084c0932e3c01c1c6815a8adb49f78c2868d4cc265d749250eb3c3b542dd9`, differs only by an AST-identical line wrap in the ARM64 recorder's dynamic schema-version message. The other 24 files, including every Rust/SQL/contract input, remain byte-identical. The full workspace result retains its original tested identity; post-wrap formatting and diff checks are separate.

## Actual local validation

The [compact gate record](local-checks.json) and retained raw captures establish:

- Workspace: **444 passed**, comprising 440 runtime/unit/integration cases and four compile-fail documentation cases; zero failed or ignored.
- Focused regressions: three domain cases, ten persistence cases and six genuine cross-library libtest cases (five behavioral tests plus one subprocess helper).
- Reference tests: 23 passed. ARM64 harness unit tests: 33 passed; these are local harness tests, not a native ARM64 runtime run.
- Formatting, strict workspace Clippy, repository boundaries, generated contracts, Git whitespace, and changed Python lint/format checks passed.
- Cached cargo-deny and cargo-audit passed. Audit scanned 227 dependencies using 1,290 advisories. The 211 registry/Git packages are unchanged; two approved existing core dev edges were added.

The genuine gzip/zstd tests retain an archive, queue a proposal, kill the child at flushed barriers, reopen the Store and recover/re-admit actual custody before fresh staging and private preparation. They also exercise exact database-only replay while custody is unavailable or mutated, full current-fact mismatches, a real SQLite audit fault after custody publication, and private prepared attempts that remain ephemeral. Owned test cleanup removes fixture directories; it does not prove an installed orphan collector.

The populated compiled-schema-8 fixture preserves sessions, grants, container/storage plans, approvals, receipt/verification payload bytes, resource requirements and held uncertain locks. It proves additive migration and transactional rollback. Its receipt strings are preserved payloads, not authenticated executor qualification. No installed older package upgrade or installed backup recovery was run locally.

## Preserved attempts and security scope

Two compiler attempts failed with ENOSPC before tests ran. A scoped Cargo cleanup removed 5.0 GiB of development outputs for five changed workspace packages; release artifacts and evidence were preserved. The third compile passed, with final chunk `af46ec` and exit 0 retained in the orchestration store. Initial recovery prose overlooked that chunk after receiving an Unknown-process response; the corrected annotation preserves the original bytes. The subsequent full workspace execution independently establishes the runtime result.

The packet retains the initial old-schema fixture setup failure, sandbox reference socket EPERM, missing Ruff executable/cache-lock errors, the interrupted reference request without an execution result, and the corrected recorder formatting failure. The original Ruff escalation passed; its retained result was recovered separately. The first passing core-focused run emitted an unused import warning; its corrected final source passed without warnings. The first deny capture truncated duplicate inclusion graphs; the complete compact rerun retains the same checks and passes.

Security checks copied the existing advisory snapshot into an owned writable cache without fetching. All 1,309 advisory/support files matched. The [identity record](local/limeos-backup-durable-advisory-identity.json) binds Git commit `ef6173cbc5c50ec8166f9a5b28f07834144373ee`, dated `2026-10-03T10:14:03+02:00`, and content identity `620ecfa835f53b6d1d8ffe9ae7de03d43c1d5e8da6927d79623cc0abbb3e8246`. The deny policy changes only its private cache path. These checks make no online freshness claim.

## Exact-source CI

[CI run 38131915369](https://github.com/Brownster/limeos/actions/runs/38131915369) completed successfully on source `8288315c245c58e88b7ca946c6a755787371f1d2`. The [compact CI record](ci-validation.json), [raw API capture](ci/ci-complete.json) and compressed job logs bind the actual completed jobs, artifact ZIP/API digests, package controls and ELF payload identities.

Both native AMD64 and ARM64 jobs passed **444 Rust cases** (440 runtime/unit/integration plus four compile-fail cases), with zero failed or ignored. Each passed the 33 ARM64-harness, eight probe/workflow-identity and 41 confinement-harness unit cases. The separate reference suite passed 23 cases. Native formatting, strict Clippy, repository/contracts, dependency and frontend gates passed. The actual CI audit logs loaded 1,296 advisories for 227 dependencies; their advisory Git/content identity is not recorded here. The local cached 1,290-advisory result remains separate.

Six Debian packages were collected: standard `0.4.4`, upgrade `0.4.4+ci.1` and shadow `0.4.4` for each native architecture. Their bytes match the exact-source artifact ZIP members and recorded digests. All four binaries in each package are 64-bit, little-endian ELF for the declared architecture. Reported installed package/binary hashes match these actual package payloads.

The existing AMD64 Debian guest job freshly produced **180 checks**: 14 foundation, six observation, 61 reference, 50 approved-storage and 49 dependency checks. The selected schema-bearing results report authority schema **9**. They retain the verified Debian image identity. These existing suites qualify their recorded package behavior; they do not exercise an installed backup recovery coordinator. No installed upgrade from an independently acquired schema-8 package artifact is claimed.

The [collection provenance](ci/collection-provenance.json) preserves the initial sandbox networking failure and subsequent approved collection separately from CI success. The raw collector summary contains an inherited `rw040-confinement-probe-integration` label; its bytes are retained, and the normalized CI record identifies this backup-binding scope. No new RW040 library or guest qualification is claimed. The 41 confinement cases are harness unit tests; all 17 combined acceptance cases remain open.

## Remaining gates and estimate

No public backup route, dispatcher, RPC, runtime service, live destination or host policy changes were made. New backup jobs remain Queued or Canceled and acquire no locks. Database bindings never authorize archives or reconstruct prepared owners. Production inventory, trusted selection metadata, full protected workspace policy, physical dependencies and quotas still need an integration source.

BKP-001, installed coordination/recovery, live restore, P04 effects and cutover remain open. The previous storage-inventory budget result remains **61.401 ms p95 against 20 ms**, failed. No local guest, Pi, wybie, SSH, workstation Docker or frozen Python testing was run for this increment. The remote existing Debian suites and native CI results are recorded separately above; native ARM64 CI does not establish Pi or bare-metal qualification.

The [recorded estimate](../../../plans/2026-10-11-backup-durable-preparation-design.md) is 32 human engineering hours, with scope review at 48. P04 retains 320 hours and its 480-hour review threshold. Measured cumulative human effort is unavailable, so the phase threshold status is unknown. Agent time, wall time and infrastructure waits are not recorded as human effort.
