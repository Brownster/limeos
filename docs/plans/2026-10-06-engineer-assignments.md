# Active engineer assignments

Both assignments start from integrated baseline `a595c2f1bcca8a8a0bc21ea0162eea841d0d5d15`. Use separate branches/worktrees and return small reviewable commits; the integrator merges shared runtime changes sequentially. The [integration record](../rewrite-evidence/p04/2026-10-06-handoff-integration/README.md) distinguishes the tested runtime `8acac40` from subsequent documentation and qualification tooling.

| Owner | Assignment | Branch | Exclusive ownership |
|---|---|---|---|
| Engineer 1 | [Overnight native ARM64 qualification](2026-10-06-engineer-arm64-overnight-qualification.md) | `engineer/arm64-overnight-qualification` | ARM64 harness and new native evidence; sole Pi window user |
| Engineer 2 | [Verified backup replay and private staging](2026-10-06-engineer-backup-verified-staging.md) | `engineer/backup-verified-staging` | Archive adapter, narrowly related backup domain types/tests, staging fixtures and evidence |
| Integrator | RW-040 protected physical dependencies, then coordinated mount/fstab operations | `main` / integration branches | Storage runtime/core/protocol/API/packaging integration; no archive-adapter edits while engineer 2 works |

The operator authorized `holly@wybie` until **07:00 on 2026-10-07, Europe/London (`06:00 UTC`)**. Engineer 1 must establish bounded host-side guest shutdown before running qualification. All LimeOS installation, disk writes and fault injection stay inside disposable guests; Python and production host services/data stay untouched. Default wybie exclusion resumes after the window. Engineer 2 and the integrator do not use the Pi concurrently.

Each assignment estimates 24 engineering hours and reviews remaining scope at 36 hours. Hardware time expires independently of those estimates. P04 remains estimated at 320 hours with cutover-scope review at 480. Record agent time, engineering time and infrastructure waits separately. Native performance and BKP-001 stay open until their required evidence exists.
