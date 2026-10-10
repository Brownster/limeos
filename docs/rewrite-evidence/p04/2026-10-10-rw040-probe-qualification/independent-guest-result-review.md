# Independent exact-46f7 guest result review

**GO for honest publication of this bounded standalone evidence. No combined, installed-effect, BKP-001 or cutover gate closes.** This review reads retained evidence only; no guest, tests, source gates, host process commands or policy changes were performed by the reviewer.

Actual source: `46f7daadcc5d930ca97925d96f05baddb20e1b2f`; artifact CI: `38076433426`; guest run: `20261010T185555Z-42f25d03`. Input provenance was independently accepted in `/tmp/limeos-rw040-46f7-input-review.md`. The reviewed genuine public probe binary is `523f6e94573e975c6f2410bc641573d4e836f0bf62a1a5a90559e66271de9268`; probe source and package/image/harness identities in the result match that exact-source input review.

Evidence root: `/home/marc/Documents/github/lime-os/target/rw040-u0-b0-46f7/attempt-02`; final assembled packet: sibling `standalone-evidence`. Read original run/done/stage/platform/service/confinement/command/environment/budget/self-check/case records, all ten raw libtest outputs and both library-baseline JSON records, plus final summary/identity/teardown records and README.

## Exact result classification

All ten executables report libtest exit 0 and exactly one final event; refusal is a valid typed test result, not successful operation availability. Every raw prefixed JSON event matches the corresponding retained event record, and all ten original stdout byte counts/hashes match. Four runs produce successful selected owners; six produce typed refusals. Seven standalone records have verified per-run closure (including three accepted refusal records); three raw refusals remain unaccepted because their cgroup identity was never observed.

| Membership | Case | Mode | Actual final status / stage | Standalone record |
|---|---|---|---|---|
| Empty | Engine | U0 | Success / `owners.revalidated` | Accepted, closure verified |
| Empty | Engine | B0 | Engine `unavailable` / `engine.collect` | Unaccepted, cgroup unobserved |
| Complete | Engine | U0 | Success / `owners.revalidated` | Accepted, closure verified |
| Complete | Engine | B0 | Engine `unavailable` / `engine.collect` | Accepted refusal, closure verified |
| Complete | Engine/processes | U0 | Storage `unavailable` / `processes.collect` | Accepted refusal, closure verified |
| Complete | Engine/processes | B0 | Engine `unavailable` / `engine.collect` | Unaccepted, cgroup unobserved |
| Complete | Engine/sources | U0 | Storage `unavailable` / `sources.collect` | Accepted refusal, closure verified |
| Complete | Engine/sources | B0 | Engine `unavailable` / `engine.collect` | Unaccepted, cgroup unobserved |
| Complete | Storage inventory | U0 | Success / `storage.revalidated` | Accepted, closure verified |
| Complete | Storage inventory | B0 | Success / `storage.revalidated` | Accepted, closure verified |

Independently compared the complete U0 Engine declarations with retained ground truth: all 14 resource/image/start/running facts agree, and all 12 running PID bindings agree. The empty comparison is complete and empty. Original Engine `observed_at` remains unchanged through final delivery; actual final ages are 1.3473 seconds (empty) and 1.7987 seconds (complete), within the five-second bound. Storage final outputs come from the separate storage owner path and do not establish combined source/process admission.

U0 process and source requests refused despite successful U0 Engine and separate standard-tool access. Their generic typed outputs do not identify the precise internal failure. Do not attribute them to confinement alone or claim a capability/Group/seccomp adjustment would resolve them. B0 standard-tool `openat2` ENOSYS and proc access denials are separate evidence. Fresh diagnostic variants removing Group or RestrictSUIDSGID were explicitly deferred, and only U0/B0 ran.

## Harness, metrics and closure

The exact closed 14-stage set in done.json matches run.json and the run ID. All stages passed, including recording blocked case prerequisites; all 17 combined acceptance rows remain explicitly blocked. This is not 17 passed library acceptance cases. The fixture reached 65 total/62 running consumers and removed 51 added consumers; that is fixture feasibility, not a 65-consumer public-library budget/headroom qualification.

Installed reader bytes retain SHA `b5feec6461873dfb05aab089a429293287a5bdb82ac0eb2759a79c0217d5f352`. The recorded reproduction matches all 62 compared properties with no differences, including Group=limeos-host-access, empty capability sets, RestrictSUIDSGID=yes, 256 descriptors, 16 tasks, 64 MiB and 50% CPU quota. This compares a transient standalone probe confinement with the dormant installed unit; it is not installed RPC/worker/effect qualification.

FD/RSS/HWM/PSS/io metrics are sampled main-process measurements; memory.current/peak/events are separate whole-cgroup records. U0 sources had an observed identity and verified closure but zero measurement samples; its zero metrics mean unmeasured. Three unaccepted B0 records likewise have no usable cgroup measurement/closure proof. VM shutdown does not retroactively make these individual records accepted.

The memory self-check reached exactly 67,108,864 bytes and exited 1, with saved `oom=1` and `oom_kill=0`; it does not substantiate an OOM-kill claim. FD/task self-checks reached the configured ceilings. Installation took 534.53 seconds; raw serial also records a jbd2 blocked task, RCU stall and long clocksource/watchdog intervals. Cause is unknown. No performance, combined-headroom or Pi/ARM64 conclusion follows from this run.

The actual runner and QEMU both exit 0; recorded QEMU duration is 657.6 seconds, deadline_reached=false, and serial ends with guest filesystem shutdown and `Power down`. QEMU stderr is retained and empty. Teardown required no forced actions: the exact owned run directory was removed and saved marker scan was empty. The writer's later read-only teardown-proof records empty current owned marker/staging observations. The reviewer independently read the configured work root and found it empty; no separate process scan was run by this reviewer. This supports owned VM teardown, not a triggered deadline, parent-death, blocking-worker or scheduled-second shutdown experiment.

## Earlier attempts and packet identity

Attempt01 remains a separate failed initialization: QEMU invocation attempted, initial QMP connect raised sandbox EPERM, exit 1, no accepted library result; serial/QEMU logs were not retained on that path. Its wrapper metadata is now finalized as failed and acknowledges the missing records. The separate redirected approval request was interrupted after 201.2 seconds, with no execution session/result or owned retry state established. It is an approval wait, not a second runtime experiment. The direct escalated attempt02 is the executed run reviewed here. Failed source6964/CI38075054820 never supplied qualifying AMD64 inputs and remains separate.

The final packet has 59 retained original records plus five assembly records. Independently hashed all 64 unique paths listed by its checksum manifest: no missing, unsafe, duplicate or mismatched path. Final identities:

| Record | SHA-256 |
|---|---|
| Original attempt02 run.json | `40eb1967a79cff6a481f6cac6c32afc50694101fc19a7f4f54efa3eff86175e3` |
| Original serial console.log | `01bbd73d88e19414f0faf9a3fa21a77b15debbeed25a9b37783b608af40ce531` |
| SHA256SUMS | `526e75b683fe8e3e07881e6a74b30fde317305c33b057f37beb6dc6ebf95a4a7` |
| README.md | `bb8be3d7fb78b704e2ec87b80256d3279fd8a648e616e3364d147db27eeeca99` |
| standalone-summary.json | `aa01edacda9efb85b69510f10d80108044961e59343e9f0912fd0fc43eb3fd8d` |
| identities.json | `98254c4b26ddd919e529b497d4a9b14ca5b63bf44de9f52ca7c81ef1995400df` |
| teardown-proof.json | `dad825d78cd67e6ebb41c8bc76976fd91b6ccd5a9eb24acbfe9a18260c9d987d` |

The final README/summary accurately state these boundaries and the observed warnings. Publication GO applies to this packet and classification. All 17 combined/transition cases, worker/dependency/headroom work, P04 effects, BKP-001 and cutover remain open; production unit/capability policy remains unchanged.

Integrity clarification: the writer initially reported manifest SHA `5d4b66480851197cab0bd08e8236ad5ccd4efa9e1575bcd7c9d669cd6fe75f28`. Before this publication GO, README/summary were corrected to explicitly include the already retained RCU stall and long clocksource/watchdog warnings alongside jbd2. The corresponding final manifest is `526e75b683fe8e3e07881e6a74b30fde317305c33b057f37beb6dc6ebf95a4a7`; this final packet, not the initial manifest, is the one reviewed and accepted. A subsequent finite integrity read confirms all 59 retained raw records still match their recorded original/retained hashes and sizes, and gzip-decoded bytes match original files exactly. Execution bytes and result classification were unchanged; only documentary timing observations and their checksum entries changed. No source, test or guest rerun occurred.
