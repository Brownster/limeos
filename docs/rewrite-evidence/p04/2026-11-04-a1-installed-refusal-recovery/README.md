# Current packages stop at Docker task churn before mount planning

Source `e417751ba57a3945fb47e60d513c087f75d5da7b` supplies a fresh AMD64
build of all five executables and standard/shadow packages
`0.4.12+gateabort.1`. All 740 product inputs match the committed source before
and after the frontend, Debian build, packaging and VM phases. There are no
test-only differences between the package build and guest fixture. The fresh
frontend qualification passes 244 tests and the checked TypeScript/Vite build.
The earlier [native recovery packet](../2026-11-01-a1-gated-refusal-recovery/README.md)
qualifies the same implementation; its Rust results were not rerun here.

The offline Debian builder completed successfully using its recorded immutable
Rust 1.88 image. Library checks pass. Core is 7,679,256 bytes, the executor is
6,118,552 bytes and the stack reader is 1,278,816 bytes; both sealed-image
executables remain below 8 MiB. The local repository's signature was verified
with disposable public fingerprint `8DFD497B4C382A6F63F380E116C41C34F498D698`.
These are local qualification packages, not a published release.

The exact committed canonical runner booted a disposable Debian AMD64 guest
with 1,024 MiB RAM, two vCPUs and four empty virtual storage disks. All 31 guest
modules are frozen. The wrapper adds only a bounded stdout copy of the same
single post-failure execution observation, with no extra reads or changed
assertions. That copy was not reached in this run.

The installed guest passes **50 prerequisite checkpoints**. These include signed
installation, protected executable/library identity, schema 14, installed readers,
guided storage previews, fresh swap exclusion, approved target preparation,
durable claim enforcement, actual executor/Core deaths, independent target
reconciliation and shadow-removal isolation. Private fault fixtures remain
distinguishable in the original output. These checkpoints are not 50 new tests
or acceptance of the later mount path.

All three negative mount API cases pass without effect. Udev setup finishes
with no remaining children. Docker startup completes four fixed read passes and
40 proc observations, reporting 3.0266 quiet seconds in 4.0165 seconds. The real
positive mount plan then returns HTTP 409 at `human_plan_approval_queue`:

- Root reports `storage_mount_consumer_tasks_changed` during
  `scan.recheck.user_tasks`: zero removed tasks and one added task, PID 7055,
  TID 8386, start tick 27194, namespace identity `(4, 4026531841)`.
- That PID equals the real fixture Docker socket's authenticated root peer,
  PID 7055. The earlier startup observation records parent start tick 25532.
  This identifies this run's changing process; it does not identify every
  earlier task/FD refusal or the precise reason Docker created the task.
- The scoped positive-flow HTTP session completes one request with no
  reconnect. No positive plan, approval, action or job is recorded. Execution,
  the guardian's new `not_started` outcome, mount verification, same-key replay
  and mount restart recovery are not exercised.

This is a correct fail-closed refusal under the current predicates and a
continuing availability barrier. Startup quietness supplies no later authority.
Do not filter Docker/Core, ignore missing tasks/FDs, stop normal collectors,
extend original deadlines or retry a failed action to obtain acceptance.
Normal complete-consumer admission under task/FD churn needs a structural design
before another installed candidate. The
[gated refusal recovery](../../../p04-a1-gated-refusal-recovery-handoff.md)
remains locally qualified and installed-unproven.

The original VM guard ran **22:23:24–22:28:00 UTC on 4 November**, exit 1,
unchanged inputs and no sampled memory stop. Its minimum host availability was
8,203,149,312 bytes. The lowest sample across this packet's phases was
7,468,253,184 bytes, above the unchanged 3-GiB floor. VM admission required
4.5 GiB; compilation retained its 4-GiB admission. All scratch stayed on
repository disk. No unrelated process was stopped.

Native observations join the original wrapper, runner and QEMU start ticks to
the actual command and disposable disk paths. All eight unique recorded
qualification PIDs are absent, the owned build container is absent, no matching
disposable signer remains and VM scratch was removed. A successful service-stop
command is not treated as historical mount or helper-quiescence proof.

`source-match.json`, `failure.json` and `admission-diagnostics.json` retain the
source and failure joins. Compressed records preserve the original frontend,
build, package and VM output, input maps, failure/startup diagnostics, public
repository metadata and cleanup observations. `reproduction-sources.tar.gz`
contains the exact fixture, runner, builder, signing, guard and observer sources.
The guest's absent whole-source string is resolved by the external build's
committed input maps and matching installed package/binary hashes.
`SHA256SUMS` covers every other packet file. Private keys, packages, binaries
and disks are excluded.

A1/B1/P04 remain open. No installed UI, package upgrade/return, current ARM64/Pi,
soak or P05/P06 acceptance is added; the existing inventory-performance failure
is unchanged. Holly's Pi and the frozen app were untouched. Nothing was pushed.
