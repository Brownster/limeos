# Verified staging and overnight ARM64 integration

Both engineer deliveries were reviewed and merged sequentially into main without rewriting their commits or proof files. Backup merge `00125de36f6b5d9aecce99608e874b6a73f77157` precedes ARM64 merge `e48831f9d5fd57bfad9611b09a8bfc5655e0f65d`. The original branch tips are `edfdab9e3490a4250f1b8ed270d0c8f78c5f02ce` and `5866c49b30b9c632ea548ab1055e650a267df950`, both from `a595c2f`. Main's prior runtime `1628fd4` and its 271-test proof remain separately attributable.

The full local integration proof is bound to `a7c73459c70b0555aff5a4f366e13c3ea063ae7f`: 291 Rust tests and 33 ARM harness tests, separately from the frozen native measurements below. Its first full CI passed both native builds and packages but failed an installed P03 fixture. The reviewed correction changes only two Python qualification files in `e330065146fdf2bcd7668a05476bb3b994a82c26`. Its separate full CI passes native AMD64/ARM64 builds and packages, 9 reference unit tests and all 180 existing installed AMD64 acceptance groups. No production host testing follows from this integration.

## Verified staging

The [interface](../../../p04-backup-verified-staging.md) captures initial trusted policy and compressed archive identity in an opaque admission snapshot. Replay uses the original inspector's bounded decoding and writes only generated flat names in attempt-owned private quarantine. Exact manifest equality, stored size/hash verification and file/directory/root fsync must succeed before callers can borrow verified readers. Cancellation, I/O faults and process-kill regressions leave no usable partial catalog. No archived path becomes a host filename or live destination.

[Original staging evidence](../rw043-staging/README.md) qualifies implementation `8d547d9`, helpers `ffef192` and that branch's 251 tests: 19 substantive new cases and one subprocess helper above its 231-test baseline. Workstation release measurements peak at about 3 MiB gzip and 5.2 MiB zstd for the tested small/64 MiB inputs. Those standalone RSS figures are distinct from service PSS, native inspector RSS and installed restore behavior.

Only two already-pinned dependency edges were added: `rustix 1.1.5` and `getrandom 0.2.17`. External versions, sources and checksums are unchanged. The preserved engineer metadata says 212 external packages; review corrects that count to **211** entries with a source field, alongside 16 local workspace entries and 227 packages overall. This count correction does not change the graph or dependency qualification.

**BKP-001 stays open.** Protected live destinations, fresh approval, complete claims, policy-owned modes/ownership, snapshots, receipts, service coordination, atomic replacement and independent post-restore recovery remain integration work. Production mappings and limits still require P00 inventory.

## Frozen native ARM64 result

The [overnight report](../../arm64/2026-10-07-current-8acac40-overnight/README.md) and [recorder output](../../arm64/2026-10-07-current-8acac40-overnight-recorded/qualification-status.json) retain the native Pi 5 KVM result. Runtime source is `8acac403e32d3692b028a036a11f4fead2050580`, authority schema 8 and qualification package version `0.4.7+arm64.1`. Successful build fixtures are `d46d7ee`; the rebuilt bundle's runtime/frontend hashes match the original. The first build attempt and its missing-git harness failure remain preserved.

| Observation | Frozen native result |
|---|---|
| Rust build/source gates | 231 tests, fmt, strict Clippy, contracts, boundaries, native standard/shadow packages and signed repository pass |
| Fresh installation | 20 of 20 checks |
| Storage | 50 of 50 checks |
| Containers | 38 of 38 checks |
| Idle base-service memory | 10.44 MiB PSS, zero service swap |
| Idle CPU | 0.051% of one core over 600 seconds |
| Writes | 4.72 MB/day extrapolated from a 600-second process-counter interval |
| Full cold readiness | Maximum 456.58 ms over five starts |
| Full storage-inventory p95 | **61.401 ms against 20 ms: failed** |

Genuine schema 7 and schema 6 upgrades each pass in their own disposable guest. Schema 7 uses original source `f39396b` and package SHA-256 `31e8eb3bc92afb713ae7669a16661e9073a6834453265dea4548c4813f4b001b`; schema 6 uses `5848bce` and package SHA-256 `0a8651aa73afc7a2e67fe665f4c43d3a9f832e09c3104c316f105bea651c6e51`. Disposable repositories were re-signed around unchanged package bytes. Sessions, authority/queued state, receipts and claims survive; replay returns the original job. Current-source CI's two revision labels remain separate from these genuine historical upgrades.

These results measure native guests with 4 KiB pages, not bare metal under the host's 16 KiB-page kernel. Pi 4/SD, matched Python performance, 24-hour write/week-long package soaks, eight disks, real media and assistant workloads remain unqualified. The frozen native result does not measure later filesystem-binding or staging code.

## Inventory budget decision

The [unchanged recorder budget](../../arm64/2026-10-07-current-8acac40-overnight-recorded/budgets.json) records inventory p95 61.401 ms against 20 ms and `false`. The architecture labels the row as process startup, while this recorder measures a complete command including device probing. That scope mismatch does not turn the observed command into a passing result: direct probing belongs in full-command latency. The ceiling and historical failure stay unchanged. All footprint rows cannot be called passing.

The original report's attribution to `lsblk` is an erratum: this source reads sysfs and runs `blkid` against every retained block descriptor twice. No profile isolates probing cost. The measured full-command time stands, while the explanation of its components remains unmeasured.

A future startup-only measurement and a separately specified probing workload can clarify performance costs prospectively. Neither was performed here; no optimization or revised ceiling is part of this integration. The previous approximately 52 ms command observation is context, not acceptance. The matched Python experiment and larger disk workload remain open.

## Expired host window and teardown

The named window ended `2026-10-07T06:00:00Z` (07:00 Europe/London). The last guest stopped `2026-10-06T23:03:35Z`; the final teardown observation was `23:07:31Z`, before the scheduled collection point and hard cutoff. The supervisor deadline proof, retained first attempt and post-run timestamp correction are individually attributable in the original report.

The engineer's [host-package decision](../../arm64/2026-10-07-current-8acac40-overnight/host-package-decision.json) records a claim of an operator amendment to install QEMU prerequisites. The repository has no independent operator transcript proving that exception. Exactly 41 packages were added and subsequently purged; the before/after 717-entry package snapshots match. This establishes package-list restoration, not a completely unchanged host. Build guests also caused up to 209 MiB of host swap. Future host sizing needs fresh authorization and pressure assessment.

The Docker shutdown's caller remains unidentified in the retained evidence. The user has taken over production container recovery. The integration worker stopped after one UTC time/container-status read, with no starts or other host writes; that observation stays private outside the repository. No further host SSH, host guest run, host test, benchmark, package action or frozen-Python development ran for this integration. Default wybie rejection applies outside a current authorized window; the historical window grants no future access.

## Corrected runner proof

Post-review commits `0bceac3`, `e0bb3fa`, `1ca6e89` and `a7c7345` fix expiry checks at every host/guest SSH/SCP dispatch, bound local transport timeouts, replace numeric-PID stop with exact-identity pidfd signalling, and establish a detached root-owned guest owner before foreground QEMU can run. Private root-owned/hash-verified inputs, cleared environments and isolated Python imports preserve the root execution boundary. The owner retains its child/pidfd, kills and reaps on publication/bind/log failure and survives loss of its simulated SSH parent before identity publication. Independent review checks clock crossing during prelaunch work and both daemonize spellings.

The initial expiry diagnostic combines exact `e48831f` runner code with later regression tests and demonstrates two expected failed assertions for SCP dispatch after expiry. Its main-path tracebacks identify the regression tests loaded using `runpy`; they do not make it a historical native run. The raw failure, current passing harness output and independent review are retained under this record. An earlier reproduction setup had an insufficient directory depth; its overwritten setup log is unavailable and is not presented as recorded proof.

This is local process/mock proof and current-source CI scope, not a new native QEMU launch qualification. The historical native run used its earlier supervisor. Expiry/timeout prevents new dispatch and bounds workstation transports; it does not remotely terminate arbitrary already-dispatched host commands or background effects. Only this guest owner's lifecycle has the independent shutdown proof. Another native launch requires a newly authorized isolated host; none ran here.

## Current integration checks and remaining gates

Actual local totals on `a7c73459` are **291 Rust tests, zero failed/ignored**, and **33 ARM harness tests**. One of the Rust additions is a subprocess crash helper, identified separately in the engineer's evidence. The original delivery's 18-test harness proof remains historical. Formatting, strict all-target Clippy, contracts, repository boundaries, cached offline cargo-deny/audit, current harness Ruff lint/format and ShellCheck pass. An optional broader Ruff attempt found style/executable-bit issues in the immutable historical staging measurement helpers; its failed output and exact scope remain retained, and those proof files were preserved. It is separate from the passing required current harness checks. Unchanged frontend/browser/release/package/installed gates pass in the corrected-source full CI rather than a redundant local frontend rerun.

Native current-source CI/package and existing isolated AMD64 acceptance retain their new source identity; they do not qualify new installed restore or combined root collection paths that have no executable integration yet.

[Check metadata](checks.json) binds exact command output, outcomes and counts to `a7c73459`. Compressed transcripts retain the captured UTF-8 tool output bytes without trimming or terminal-newline changes; recorded observation times describe check collection, not program-startup benchmarks. [The original source manifest](source-manifest.json) binds 276 runtime/fixture/frontend/release inputs to that commit's Git blobs and their bytes at collection. The fixture correction legitimately changes one of those files and adds a regression file: [the final source manifest](final-source-manifest.json) separately binds 277 inputs to `e330065`. The original manifest remains a historical snapshot, not a claim that every current filesystem byte still matches `a7c73459`. [The preserved-proof manifest](preserved-proof-manifest.json) verifies 160 original engineer evidence files unchanged. [Independent runner review](runner-independent-review.md) and [its hash metadata](runner-independent-review.json) retain the separate three-boundary mocked check and implementer's 33-case proof. `SHA256SUMS` covers this record's files and excludes itself.

## Installed fixture failure and correction

[The first full CI](https://github.com/Brownster/limeos/actions/runs/37608091754) tested exact `a7c73459`. Native AMD64 and ARM64 each passed 291 Rust tests, 33 ARM harness tests, release/package gates and their applicable browser checks. The disposable Debian VM passed P01 (14 checks) and P02 (6), then failed the original P03 crash fixture: the next plan returned 503 after `limeos-containerd` was killed. Both P04 guest groups did not run in this attempt. [The retained result](ci-attempt-1/summary.json) binds full logs and all six native package identities. Only the three fresh VM artifacts are selected; older committed results in the uploaded bundle cannot establish acceptance for this source.

[Independent diagnosis](ci-attempt-1/independent-diagnosis.md) traces the race to the fixture's `systemctl is-active` observation. The killed unit's old active state and a terminal interrupted job can precede the automatic restart and its IPC listener. A plan requiring fresh inspection correctly returns unavailable during that interval. The failed attempt aborts before the last executor restart; it does not prove eventual recovery of that crash. Earlier restarts and the frozen native run remain supporting context rather than replacements for the new installed gate.

Commit `e330065` requires genuine framed health IPC as the authorized `limeos-core` UID/group before the crash loop proceeds. It preserves the existing 45-second recovery deadline, all expected job states, one-effect assertions and prepared receipts. The reused probe now reads fragmented frames exactly, rejects truncated EOF and responses outside 1–65,536 bytes, and bounds each subprocess and poll sleep by the remaining deadline. POSIX CPython's `subprocess.run` kills and waits for its direct child on timeout; the initial speculative post-kill captured-pipe hang was withdrawn after checking the Debian Python implementation. This fixture check makes no claim about remotely terminating arbitrary descendants or background effects. No runtime retry or service behavior changed.

[Fixture evidence](fixture-fix.json) retains the instrumented original-condition diagnostic (two failing cases, one passing), the separate EOF/unbounded-probe diagnostic (two failing, three passing), and the corresponding passing outputs. Full reference discovery on the final correction passes **9 tests: 7 new regressions and 2 existing cases**. The independently run ARM harness passes **33 tests**. Final required Ruff lint/format, contracts, repository and diff checks pass; intermediate local lint findings are retained with their corrected passing results. [Independent fixture review](fixture-independent-review.md) and [its source/log hashes](fixture-independent-review.json) bind the exact corrected files. Unchanged local Rust/Clippy/dependency checks were not redundantly rerun; their original `a7c73459` attribution remains intact.

[The corrected-source full CI](https://github.com/Brownster/limeos/actions/runs/37634463402) passes on exact `e330065`. Each native architecture passes **291 Rust tests, zero failed/ignored**, and **33 ARM harness tests**, with fmt, strict Clippy, deny/audit, contracts/boundaries, unit definitions, frontend gates, deliberate release-gate rejection probes and Debian 12 release/package builds. AMD64 also passes the browser regression. The installed VM job separately discovers **9 reference unit tests**; these are not an inferred combined harness count.

| Fresh installed AMD64 suite | Passed groups |
|---|---:|
| P01 foundation/package-revision lifecycle | 14 |
| P02 observations/shadow isolation | 6 |
| P03 lifecycle/logs/Compose/reference layouts | 61 |
| P04 approved storage/readiness/claims/recovery | 50 |
| P04 container dependencies | 49 |
| Total | 180 |

The P03 result retains all six original core/executor crash scenarios and proves that the previously interrupted executor recovers before the next plan. Both P04 groups now run and pass. [Completed CI metadata](ci-attempt-2/summary.json) retains the three full job logs, six package hashes/control records and ELF architectures, and only the five fresh suite outputs. [The verified outcome](ci-attempt-2/verified-outcome.json) checks raw log/result hashes, all reported installed package/binary hashes against the current native packages, and a single recorded Debian image digest across the five guests. P01/P02 results do not contain per-binary hashes; their acceptance is established by this exact-source workflow and signed-repository installation logs. Older committed artifacts are excluded from the verdict. Both revision labels in this run contain this source's binaries; they do not replace the frozen genuine schema 6/7 upgrade proof.

The successful CI remains source-bound to `e330065` after documentation commits. Its acceptance scope stays separate from the frozen `8acac40` overnight measurements, hardened-launcher native qualification and unfinished live restore/combined collection paths.

RW-040 combined installed collection, Engine reacquisition, running namespaces, pool/protection/share propagation, bounded service admission, complete claims/ceilings and effect-time verification remain prerequisites. Mount/fstab, unmount/loss, live restore and the remaining parity work stay gated. No defect-register row closes on these handoffs.

Each engineer assignment retains a 24-hour estimate and 36-hour review threshold. Reported agent wall intervals and infrastructure waits do not establish human engineering hours. P04 remains estimated at 320 hours, with cutover-scope review at 480.
