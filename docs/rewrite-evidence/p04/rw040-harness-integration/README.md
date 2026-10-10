# RW-040 harness integration — local checks only

2026-10-10. Reviewed source **`74678fd7b2cf46b7404046fb8cead03415a958e4`**
on `integrator/rw040-harness-fixes`, from the engineer's
`841f0655852b1818c0978bb0b8b8c59ee661f6cc`. Main was untouched by this worker.

**41 local harness tests pass**, together with Ruff 0.16.10 lint/format,
repository boundaries, contract regeneration and diff checks. The [final
record](local-gates/record.json) binds commands, Python version, raw logs,
timestamps and exact source hashes. Inputs remained unchanged during capture.
The [independent review](independent-review.md) gives the narrow source/evidence
GO for future U0/B0 standalone baselines.

The runner now requires stable owned process identity, verified archive
completion and the unchanged producer protocol before accepting a fresh
qualification record. It preserves ambiguous ownership or exit. The result
receiver rejects extension headers before payload reads, bounds member count,
checks complete data/padding/trailer, validates run/stage identity and publishes
only a fresh complete result directory. The probe receiver requires bounded
prefixed events and successful executable/libtest exit. Provisional collection
does not qualify success; stale delivery or renewed timestamps refuse.

The regressions cover parent adoption/prctl failure and privileged exec images,
held-pidfd exit races with numeric signalling forbidden, unknown/denied/pending
ownership, archive extensions/duplicates/count/truncation, missing completion,
malformed/truncated/duplicate/late events, omitted selected owners, conservative
fractional age, real nonempty serialized instance/PID shape and explicit
measurement ambiguity. The exit-race test uses a distinct live replacement; it
does not claim an actual kernel PID-reuse event.

The earlier [40-test capture](local-gates/attempt-40-before-delivery-bound/record.json)
is preserved. It preceded the final conservative monotonic delivery bound and
its extra timestamp-free-storage regression. [Earlier tool attempts](attempts.md)
record pre-capture failures and the aborted approval honestly; none is counted
as final qualification.

All **111 entries** in the frozen engineer evidence still verify; their exact
identities are retained in [identities.json](identities.json). The historical
14 tool stages and 17 blocked acceptance rows are unchanged.

No guest, workstation Docker, SSH, wybie or Pi operation ran. No Rust source,
unit, capability, ceiling, workflow or frozen Python change is included.
Fresh guest execution awaits a genuine Debian-12-compatible, same-library-source
probe/package supply. Only U0 unrestricted control and B0 unchanged installed
confinement are in scope. The 17 combined/transition acceptance rows, protected
dependency-contract setup, worker supervision, combined capacity, destination
mapping and all P04/defect/cutover gates remain open. Standalone measurement is
not combined-worker headroom.

The [owned harness README](../../../../tests/qualification/rw040-combined/README.md)
and [supply record](../../../../tests/fixtures/rw040-combined/probe-contract.md)
describe the implemented interface. The historical draft remains explicitly
marked as superseded by the published producer contract.
