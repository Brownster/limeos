# A1 reaches execution resource admission

The corrected candidate uses source
`b1f747de43d537f6260faee7ad151cef591d7cb1`, schema 12 and disposable standard/shadow
packages `0.4.8+runtime.1`. The offline Debian build, current UI build and signed
repository preparation passed. All 687 recorded product inputs, guest modules
and installed binary/package hashes join that source.

The fresh 384-MiB guest passed initial collection, all three negative pre-effect
API cases, genuine positive plan creation, human approval and durable job
queuing. The mount executor starts with one thread; no task-difference event
was recorded. This moves past the [idle-worker refusal](../2026-10-29-a1-executor-idle-thread/README.md)
without weakening host-task comparisons.

Execution then refused at `inspect.capacity.admit` with `Unavailable`.
The subsequent protected receipt lookup returned `NotFound`; Core retained the
job as `needs_intervention`. The fixture failed at `await_terminal_receipt`
and never retried the action. No successful mount, independent mounted proof,
positive replay/restart or installed UI acceptance is credited.

The exact resource subcondition is not logged. Post-refusal observations show
one thread, 7,320 KiB resident memory and ten open descriptors, but those later
values cannot reconstruct execution admission. The source opens the protected
`MountJournal` before `inspect`; inspection-only requests have no such retained
owner. Its SQLite WAL connection, protected directory and process lock consume
descriptors that must be measured and accounted for explicitly. The current
capacity model admits a baseline of at most 16 descriptors. This establishes the
next investigation; it does not prove the unrecorded failing descriptor count.

This run has a complete original terminal guard record: started
`2026-10-29T22:56:15.618868+00:00`, ended
`2026-10-29T23:01:03.298251+00:00`, exit 1, unchanged source inputs and no
sampled memory stop. Minimum observed host availability was **6,303,768,576
bytes**, above the unchanged 3-GiB stop floor. The launcher detached the guard
into its own recorded process session and disk log. Native observations confirmed
that guard live during the run and absent after terminal completion. The runner
removed its disposable scratch directory. The earlier missing-terminal run
retains its separate diagnostic-only status.

`a1-failure.json` preserves the actual job, service, negative cases and unit
diagnostics. Raw guard/build/package records, source joins, admission and launch
records, startup diagnostics and the exact harness are retained. The guest and
runner modules are compacted in `guest-and-runner-sources.tar.gz`. Source copies
retain the runtime bootstrap, execution ordering, admission model and schema.
Private signing keys, binaries, packages and VM disks are excluded.
`SHA256SUMS` covers every other packet file.

Measure the genuine journal's retained descriptors in an isolated native process,
then account for execution-only ownership within the existing aggregate FD model.
Preserve replay-before-execution, cross-process locking, original owner lifetimes,
service ceilings and uncertain-job barriers. Use a new candidate and empty disks
for installed acceptance. The broader A/B, UI, ARM64, assistant and migration
gates remain open. Holly's production Pi was untouched and nothing was pushed.
