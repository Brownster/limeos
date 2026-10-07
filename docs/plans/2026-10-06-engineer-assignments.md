# Engineer assignments and handoffs

Both assignments started from integrated baseline `a595c2f1bcca8a8a0bc21ea0162eea841d0d5d15` in separate branches/worktrees. The integrator merged the shared runtime changes sequentially. The [original integration record](../rewrite-evidence/p04/2026-10-06-handoff-integration/README.md) distinguishes the tested runtime `8acac40` from subsequent documentation and qualification tooling.

Both deliveries are reviewed and merged with their original histories. [The new integration record](../rewrite-evidence/p04/2026-10-07-handoff-integration/README.md) separates frozen native qualification from current-source validation and preserves the failed inventory latency row. Further harness safety corrections and validation are recorded there before reuse. The original branches and evidence remain intact.

| Owner | Assignment | Branch | Exclusive ownership |
|---|---|---|---|
| Engineer 1 | [Overnight native ARM64 qualification](2026-10-06-engineer-arm64-overnight-qualification.md) | `engineer/arm64-overnight-qualification` | ARM64 harness and new native evidence; sole Pi window user |
| Engineer 2 | [Verified backup replay and private staging](2026-10-06-engineer-backup-verified-staging.md) | `engineer/backup-verified-staging` | Archive adapter, narrowly related backup domain types/tests, staging fixtures and evidence |
| Integrator | RW-040 protected physical dependencies, then coordinated mount/fstab operations | `main` / integration branches | Storage runtime/core/protocol/API/packaging integration; no archive-adapter edits while engineer 2 works |

The operator authorized `holly@wybie` until **07:00 on 2026-10-07, Europe/London (`06:00 UTC`)**. The original brief required engineer 1 to establish bounded host-side guest shutdown before qualification, confine LimeOS installation/disk writes/fault injection to disposable guests and preserve production code/services/data. Engineer 1 alone owned that window; the integrator and engineer 2 did not run qualification on the Pi.

That window has expired. The last guest stopped at `2026-10-06T23:03:35Z` and the final teardown check was at `23:07:31Z`. The engineer recorded an operator amendment for temporarily installing 41 QEMU-related host packages; the repository contains that claim, not an independent operator transcript. The package snapshots show exactly 41 additions, their purge and restoration of the original 717-entry package list. This does not establish a completely unchanged host. No further host testing is authorized by the historical window.

Each assignment estimates 24 engineering hours and reviews remaining scope at 36 hours. Hardware time expires independently of those estimates. P04 remains estimated at 320 hours with cutover-scope review at 480. Record agent time, engineering time and infrastructure waits separately. Native performance and BKP-001 stay open until their required evidence exists.
