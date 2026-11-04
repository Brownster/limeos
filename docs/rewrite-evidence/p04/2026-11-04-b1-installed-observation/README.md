# Installed observation schedule reaches no first settlement

Fixture source `f3be3b3de1a34d1aec05e9ae120a5c1f3864e987` implements the
[installed B1 design](../../../plans/2026-11-04-b1-installed-observation-qualification-design.md).
It adds a standalone disposable-guest scenario and five native boundary cases.
The registered production timer, policy, protocol, grants, schema and service
ceilings are unchanged. **The installed scenario fails at its first incident
wait; no successful installed health collection is established.**

The fixture uses the same signed AMD64 packages `0.4.12+gateabort.1`, built at
`e417751ba57a3945fb47e60d513c087f75d5da7b`. All 740 original product inputs
match. The 742-input fixture map adds only the two new test files; both added
hashes and the separate fixture/product commits are explicit. All 30 guest
modules and the exact canonical non-build runner are frozen. Package, executable
and schema-source hashes join the actual installed result to the
[original package build](../2026-11-04-a1-installed-refusal-recovery/README.md).
Its 244 frontend passes and Debian build were not rerun here. Neither package
version text nor a branch name alone supplies this provenance.

Both scoped native suites pass: five new fixture cases and 13 reused mount
fixture boundary cases, **18 cases total**. They establish the pre-effect
root/hostname guard, bounded failure output, SQLite handle closure with GC
disabled on success/error, exact occurrence/job/result/channel bindings and
original recovery generation. Scoped Ruff, formatting, repository and diff
checks pass. The first guarded attempt passed both suites, then failed lint;
the corrected second attempt and the original failure are retained. Native
fixtures do not establish installed trust or timer execution.

The canonical 1,024-MiB Debian guest passes signed installation and independent
schema-14 verification from delivered Rust source. A real packaged root Docker
daemon owns the Unix listener; no HTTP proxy or synthetic health server is used.
The fixture creates one actual full-ID container with `unless-stopped` policy,
initially not running. Bootstrap/enrollment, password-verified self-enablement
and relogin use the actual installed paths. Missing management authority, CSRF,
Origin and authentication cases refuse without changing observation history.
The supported health policy is set to fail threshold one through its POST API.

The installed reader ceiling is retained byte-for-byte and the dormant reader
is explicitly started only inside this guest. Its real Core-UID storage inventory
probe reports one overloaded response, then ready after 0.178 seconds. The root
reader has empty effective capabilities, `NoNewPrivs`, the original 64-MiB
memory ceiling, 16-task ceiling and 256-descriptor limit. **That inventory probe
is not a container-health collection result.**

The authenticated schedule POST returns 201 with a real 30-second interval,
29-second grace, `coalesce_latest`, server-owned anchor and `scheduled` status.
Its initial runtime phase is `waiting`. The ordinary timer is left enabled;
there is no direct fixture settlement, clock override or mutation retry.
After the bounded 75-second incident wait expires, the read-only failure snapshot
contains the one unpaused schedule, with no settled watermark, and zero
occurrences, jobs, results, alerts or notifications. The original service journal
contains startup messages and no diagnostic identifying the refused collection
or completion boundary. No Core crash, incident/recovery, acknowledgment or pause
acceptance is reached.

The current evidence cannot distinguish protected container-health collection
failure from Core admission/completion refusal. The failure packet lacks a final
authenticated runtime view and a separate real Core-UID container-health reply.
Add bounded post-failure read-only observations for those two boundaries before
choosing a product correction. Preserve original nonces/deadlines, default
collectors, concrete scope checks and all resource ceilings. A direct later
read must remain labeled as an independent observation, never the timer's
historical reply or a successful occurrence. Do not infer Docker task churn from
the separate A1 result.

The original VM guard runs **22:42:56–22:46:04 UTC on 4 November**, exit 1, with
all 742 inputs unchanged and no sampled memory stop. Minimum host availability
is 7,850,192,896 bytes, above the unchanged 3-GiB floor. Native and VM admissions
retain 4-GiB and 4.5-GiB thresholds respectively. All scratch stays on repository
disk. Native observations show all seven unique recorded guard/runner/QEMU
PIDs absent and disposable VM scratch removed. No unrelated process was stopped.

`installed-result.json` retains the actual bounded failure; `source-match.json`
records its provenance and limits. Compressed native/VM records preserve both
guarded native attempts, frozen manifests, startup/failure diagnostics and
cleanup. `reproduction-sources.tar.gz` contains exact guest, runner, reference,
guard and qualification helper sources. The earlier package packet retains the
full build/repository reproduction. `SHA256SUMS` covers every other packet file.
Private keys, packages, binaries and disks are excluded.

B1 and A1/P04 remain open. Successful root-positive container health, installed
API/UI/restart publication, remaining health sources, reviewed replacement/
retirement, compaction, ordinary channels, current ARM64/Pi, upgrade/return,
footprint/soak and P05/P06 remain required. The historical inventory-performance
failure is unchanged. Holly's Pi and the frozen application were untouched;
nothing was pushed.
