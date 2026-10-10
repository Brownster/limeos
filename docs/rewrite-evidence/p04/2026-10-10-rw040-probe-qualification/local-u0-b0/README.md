# Actual U0/B0 standalone library baseline

The disposable Debian 12 AMD64 guest completed on 2026-10-10 with four successful
owner results and six typed refusals. Three short B0 refusals lack observed cgroup
identity and remain unaccepted records. All 17 combined acceptance cases remain
blocked.

Source: `46f7daadcc5d930ca97925d96f05baddb20e1b2f`; actual artifact CI:
`38076433426`; actual guest run: `20261010T185555Z-42f25d03`. The guest used
the genuine same-source AMD64 packages and unchanged public-library probe.
[identities.json](identities.json) binds the command, source, probe, package,
installed binaries, unit, image, harness and fixture hashes. The original verified
input and acquisition records are retained separately from execution records.

U0 runs as unrestricted guest root. B0 reproduces the unchanged installed reader
confinement, matching all 62 compared properties. Each retained result records
the fixed libtest invocation, successful executable/libtest exit, exactly one
final event, its original observation clock and result framing. A typed refusal
records an unavailable operation; it does not prove that operation works.

| Membership | Owner case | Mode | Actual final result | Closure / record |
|---|---|---|---|---|
| Empty | Engine | U0 | Success; complete empty membership | Verified; accepted |
| Empty | Engine | B0 | Engine `unavailable` at `engine.collect` | Unobserved cgroup; unaccepted |
| Complete, 14 containers | Engine | U0 | Success; instances and running PID bindings match | Verified; accepted |
| Complete | Engine | B0 | Engine `unavailable` at `engine.collect` | Verified; accepted refusal |
| Complete | Engine/processes | U0 | Storage `unavailable` at `processes.collect` | Verified; accepted refusal |
| Complete | Engine/processes | B0 | Engine `unavailable` at `engine.collect` | Unobserved cgroup; unaccepted |
| Complete | Engine/sources | U0 | Storage `unavailable` at `sources.collect` | Verified; accepted refusal |
| Complete | Engine/sources | B0 | Engine `unavailable` at `engine.collect` | Unobserved cgroup; unaccepted |
| Complete | Storage inventory | U0 | Success | Verified; accepted |
| Complete | Storage inventory | B0 | Success | Verified; accepted |

[standalone-summary.json](standalone-summary.json) contains the exact ten rows,
raw-output identities, original delivery ages, samples and closure observations.
The successful Engine results retain their original `observed_at`; final delivery
arrived within five seconds. The actual library refusals expose `unavailable`.
Separate standard-tool B0 observations record `openat2` ENOSYS and PID 1 proc
access EACCES; these do not prove the precise internal cause of every refusal.

The runner and QEMU exited 0 after 657.6 seconds. All 14 harness stages passed,
including result admission; this includes recording blocked cases and refused
operations. The deadline was not reached. The guest powered off, its owned run
directory was removed, and the recorded marker scan was empty. A subsequent
read-only observation found the owned marker and staging root empty; see
[teardown-proof.json](teardown-proof.json). No scheduled-second deadline test or
combined-worker teardown proof is claimed.

Main-process FD/RSS/PSS/io samples and whole-cgroup memory/events have different
scopes. Zero samples mean unmeasured, including the U0 sources refusal. The fresh
memory self-check reached 67,108,864 bytes and exited 1, recording `oom=1` and
`oom_kill=0`; it does not establish an OOM kill. Installation took 534.53 seconds,
and the serial log contains `jbd2` blocked-task, RCU stall and long clocksource
readout/watchdog warnings. Their cause is not established. These observations do
not qualify combined headroom or performance budgets.

Earlier evidence remains distinct: source 6964 failed to publish qualifying
packages/probe; attempt 01 invoked QEMU but failed initial QMP connect with sandbox
EPERM and produced no library result. That initialization path did not retain
serial/QEMU logs, so their contents are unavailable. A redirected escalation
request was interrupted after 201.2 seconds without an execution result or owned
retry state. It is an approval wait, not an executed guest. The direct escalated
retry produced attempt 02. Original metadata, stderr and runner output are kept
under `raw/`; [raw-identities.json](raw-identities.json) records original and
retained byte hashes, including any gzip encoding.

Only U0/B0 ran. Diagnostic policy variants stayed deferred. No production unit,
capability, ceiling, Python project, wybie or Pi changed. Combined workers,
transition acceptance, dependencies, headroom, P04 effect, BKP-001 and cutover
gates remain open. This packet was assembled from retained evidence without a
guest or test rerun.
