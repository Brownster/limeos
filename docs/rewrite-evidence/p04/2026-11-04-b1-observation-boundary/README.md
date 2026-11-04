# Independent root health read succeeds while Core settles nothing

Fixture source `0705a79173c8d12e79276c6f45e47c083dbce3c6` adds the
[bounded failure observations](../../../plans/2026-11-04-b1-installed-observation-qualification-design.md#read-only-failure-boundary-observations)
needed after the [first installed B1 attempt](../2026-11-04-b1-installed-observation/README.md).
**A genuine independent Core-UID container-health read now succeeds. The normal
Core timer still reaches no first settlement, so B1 remains open.**

The fixture reuses signed AMD64 packages `0.4.12+gateabort.1`, built at
`e417751ba57a3945fb47e60d513c087f75d5da7b`. All 740 original product inputs
match. The 742-input fixture map adds only the standalone scenario and its
native reference file; both changed hashes and separate fixture/product source
identities are recorded. All 30 guest modules and the canonical 1,024-MiB
non-build runner are frozen. Actual installed package/executable and delivered
schema-source hashes match the original source-bound
[package build](../2026-11-04-a1-installed-refusal-recovery/README.md).
No Rust product, trust predicate, recurring authority, policy or service
ceiling changes here. The original setup, timer and 75-second incident wait
retain every existing assertion.

All **22 scoped reference cases** pass: nine fixture cases and 13 reused boundary
cases, with Ruff, formatting, repository and diff checks. The added private
mock-I/O cases exercise the exact fixed probe's fragmented frame, early EOF,
zero/oversized lengths, wrong client/peer UID and original absolute deadline.
Independent sections preserve unavailable data and the original error; a
separate bounded stdout packet survives the main failure-output fallback.
The expected-red guard retains four missing-helper errors before implementation.
Its sandbox tool session finished exit 1; namespace-local child PIDs are not
presented as native process identities. These native cases establish no root trust.

The guest repeats actual signed installation, schema-14 verification, a real
packaged root daemon-owned Docker socket, stopped full-ID container, enrollment,
password-verified self-enablement, relogin and negative authority/CSRF/Origin/
authentication cases. The default dormant reader starts under its unchanged
ceiling. The authenticated schedule POST returns 201 and registered status.
After the same first-incident timeout, its final authenticated view reports
`scheduled` with runtime `unavailable`, attempt time 1793833213, an unpaused
revision-1 schedule and no settled watermark. Read-only history has zero
occurrences, jobs, results, alerts or notifications.

Only after that original failure, one independent read is made under actual
Core UID **104**, matching the installed ceiling and observed Core process.
The fixed socket's authenticated root peer is PID **6146**, matching the actual
reader unit. A fresh random nonce is sent once under a four-second absolute
deadline and five-second subprocess ceiling. In **0.06919 seconds**, the real
protected reader returns `ready: true` and a container-health cohort with:

- The exact sent nonce and Engine ID matching the real fixture daemon.
- One full-ID member matching the actual stopped fixture container.
- The expected warning/failing signal, `limeos-b1-health is not running`.
- Source sample time 1793833213 and complete container-source coverage.

This establishes that particular installed protected root-image/guardian/worker/
Engine read. The container cohort's completeness does not make overall health
coverage complete: the authenticated schedule page remains partial. The later
cohort is never supplied to Core, used to settle a job or treated as the timer's
historical reply, lease, admission or completion evidence. Incident/recovery,
Core crash/restart, notification acknowledgment and pause acceptance remain
unreached.

Post-failure Core/reader samples retain empty effective/permitted/bounding
capabilities and `NoNewPrivs`. Their cgroup memory maxima remain 128/64 MiB,
task maxima 32/16 and descriptor limits 1024/256 respectively. Observed cgroup
memory peaks are 26,664,960 and 7,544,832 bytes; recorded memory/task limit and
OOM counters are zero. These are this short scenario's later observations,
not complete workload, pressure, footprint or soak acceptance.

The successful independent root read narrows investigation to Core's actual
read/admission/completion path, without proving which boundary failed. Core's
installed unit still sets `RestrictSUIDSGID=yes`; its health client uses protected
`openat2` socket lookup. A
[previous reader-profile diagnostic](../2026-10-21-stack-host-view/README.md)
isolated a conflict between that setting and `openat2` on Debian 252. This is a
specific hypothesis for Core, not a measured Core errno in this run. Next, test
the fixed read under the exact Core profile and one-variable guest-only profile
variants before changing production confinement. Preserve current credentials,
socket identity checks and original deadlines. Do not weaken the root reader,
filter observations or infer A1 task churn from this separate failure.

The original VM guard runs **22:57:00–23:00:14 UTC on 4 November**, exit 1,
all 742 inputs unchanged and no sampled memory stop. Minimum host availability
is 8,567,963,648 bytes. Native/VM admissions retain 4/4.5-GiB thresholds and the
unchanged sampled 3-GiB floor. Native observations find all five unique recorded
qualification PIDs absent and disposable VM scratch removed. No unrelated
process was stopped; all scratch remains on repository disk.

`boundary-observations.json` preserves the independent reply/runtime/resource
facts; `installed-result.json` retains the original failure and later observations.
`source-match.json` binds their provenance and scope. Compressed guards, manifests,
startup/failure diagnostics and cleanup retain expected-red, final native and VM
attempts. `reproduction-sources.tar.gz` contains exact fixtures and qualification
helpers; the earlier package packet retains build/repository reproduction.
`SHA256SUMS` covers every other packet file. Keys, packages, binaries and disks
are excluded.

A1/B1/P04, normal timer/API/UI/restart publication, remaining health sources,
reviewed replacement/retirement, compaction, ordinary channels, current ARM64/Pi,
upgrade/return/footprint/soak and P05/P06 remain open. The historical inventory
performance failure is unchanged. Holly's Pi and frozen application were
untouched. Nothing was pushed.
