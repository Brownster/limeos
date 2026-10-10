# RW-040 exact-source CI and standalone qualification

The exact-source CI passed. The disposable Debian 12 AMD64 guest produced **four successful owner results and six typed refusals**. Seven standalone records were accepted, including three refusals; three B0 records remain unaccepted because their cgroup identity was not observed. All 17 combined acceptance cases remain blocked.

Source: `46f7daadcc5d930ca97925d96f05baddb20e1b2f`. [CI run 38076433426](https://github.com/Brownster/limeos/actions/runs/38076433426) and guest `20261010T185555Z-42f25d03` used matching genuine packages and the unchanged public-library probe. [The compact summary](qualification-summary.json) joins their identities and preserves their different scopes. This packet was assembled from retained evidence without rerunning tests or guests.

Both native AMD64 and ARM64 lanes passed **425 Rust tests** (421 unit/integration plus four compile-fail), 33 ARM64 guards, eight producer/workflow regressions and 41 console harness tests each. The installed lane passed 23 reference tests and **180 checks** across five freshly written suites (14, 6, 61, 50 and 49). Six package identities, 24 ELF identities and reported installed package/binary hashes matched the actual artifacts. [CI captures](CI-README.md), [acquisition](input-acquisition.json), [input verification](verified-inputs.json) and [independent input review](independent-input-review.md) bind the exact source, package/probe bytes, compiler, builder content ID, ELF/GLIBC and Debian image. The CI-only README retains its initial assembly wording; the actual guest outcome follows here.

U0 runs as unrestricted guest root. B0 reproduces the unchanged installed reader confinement and matched all 62 compared properties. A typed refusal means the requested operation was unavailable. The complete successful U0 Engine result matched all 14 instances and all 12 running PID bindings; the empty result matched complete empty membership.

| Membership | Owner case | Mode | Actual result | Standalone record |
|---|---|---|---|---|
| Empty | Engine | U0 | Success | Accepted |
| Empty | Engine | B0 | Engine unavailable | Unaccepted; cgroup unobserved |
| Complete | Engine | U0 | Success | Accepted |
| Complete | Engine | B0 | Engine unavailable | Accepted refusal |
| Complete | Engine/processes | U0 | Storage unavailable | Accepted refusal |
| Complete | Engine/processes | B0 | Engine unavailable | Unaccepted; cgroup unobserved |
| Complete | Engine/sources | U0 | Storage unavailable | Accepted refusal |
| Complete | Engine/sources | B0 | Engine unavailable | Unaccepted; cgroup unobserved |
| Complete | Storage inventory | U0 | Success | Accepted |
| Complete | Storage inventory | B0 | Success | Accepted |

[The standalone packet](local-u0-b0/README.md) retains all ten raw results, original clocks, sampling and closure records, with [independent result review](independent-guest-result-review.md). The actual generic refusals do not establish their precise internal cause. Standard-tool openat2/proc observations remain separate. Zero samples mean unmeasured; main-process FD/RSS/PSS/io and whole-cgroup memory/events have different scopes.

The runner and QEMU exited 0 after 657.6 seconds. **All 14 harness stages passed**, including recording blocked prerequisites and typed refusals. The guest powered off, its owned run directory was removed and marker/staging observations were empty; [teardown proof](local-u0-b0/teardown-proof.json) records the evidence. The deadline was not reached. These results qualify neither a triggered deadline nor combined-worker closure.

The fresh memory self-check reached 64 MiB and exited 1 with `oom=1`, `oom_kill=0`; it does not prove an OOM kill. Installation took 534.53 seconds; serial records jbd2 blocked-task, RCU stall and long clocksource/watchdog warnings. Their cause is unknown. No performance, combined-headroom, Pi or ARM64 runtime conclusion follows from this guest.

The [failed 6964 CI and ordering recovery](../2026-10-10-probe-producer-recovery/README.md), failed QMP EPERM initialization and interrupted 201.2-second approval wait remain distinct. The [raw identities](local-u0-b0/raw-identities.json) preserve original bytes and compression hashes, including acknowledged missing initialization logs. [CI-SHA256SUMS](CI-SHA256SUMS) preserves the earlier CI-only manifest; [SHA256SUMS](SHA256SUMS) covers the joined packet and unchanged standalone manifest.

Combined/transition acceptance, worker cancellation/deadlines, physical dependency composition, headroom, P04 effects, BKP-001, installed preparation/recovery, live restore and cutover remain open. The historical inventory p95 **61.401 ms against 20 ms remains failed**. Production units, capabilities and ceilings are unchanged. No Pi, wybie, SSH, workstation Docker or frozen Python work ran.

The [next durable backup binding slice](proposed-next-backup-integration.md) is a proposal only. Its estimate and trusted production inputs remain work to do before implementation.
