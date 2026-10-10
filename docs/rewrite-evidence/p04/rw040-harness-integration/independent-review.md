# RW-040 fresh harness source review

Reviewed 2026-10-10 by `confinement_review`, with source, Git and captured-evidence reads only. No reviewer tests, guest, Docker, SSH, Pi operation, source edit or Git mutation occurred. This follows the historical handoff and policy review in `/tmp/limeos-rw040-confinement-handoff-review.md`.

**Source GO for fresh U0/B0 standalone public-library and tool baselines.** Production confinement remains unchanged. Diagnostic property/capability variants, worker/headroom qualification and all 17 combined acceptance cases remain deferred or blocked. No P04, defect or cutover gate closes.

The prepared patch is in `/tmp/limeos-rw040-harness-fixes`, branch `integrator/rw040-harness-fixes`, based on `841f0655852b1818c0978bb0b8b8c59ee661f6cc`. At review it comprises five modified owned files and the new owned `guest/probe_adapter.py`; no historical evidence or production source/property changed. The writer preserved the earlier 40-test capture, added one meaningful late-storage regression, then refreshed the capture to 41 passing tests. I read `/home/marc/Documents/github/lime-os/target/rw040-harness-fix-proof/record.json`: all six recorded gates exit 0 (unittest, Ruff, format, diff, repository and contracts), source hashes match this review, and `source_unchanged` is true. The raw unittest log reports 41 tests and OK; its independently read SHA-256 is `49f611e7249599d627dc1b84ecca652773b73cd3b02df1c29fb8835f2958c9f5`, matching the record. These are writer checks, not reviewer execution. Final commit identity must be recorded separately.

The patch addresses the previously identified execution blockers:

- Sweep opens retained pidfds before checking saved identity, signals only descriptors, observes exit before removing staging, and preserves unavailable/mismatched/denied/pending identities. Its real regression exercises target exit between validation and signal while numeric signaling is forbidden and another child survives. It does not exercise actual numeric PID reuse.
- Parent-death bootstrap compares the exact launch parent after successful prctl, including non-PID-1 adoption, and refuses privileged executable modes or file capabilities that could clear the signal. Adoption/prctl failures use private substitutes; ordinary parent-death behavior uses a real child. Trusted executable selection is assumed, not an adversarial host executable-swap proof.
- Result admission reads fixed USTAR headers before payload, refuses extension/link/special/path records, bounds members including empty entries and all bytes, checks exact payload/padding/two-zero EOF/trailing zero fill, and publishes only a fresh transaction after validating this run's exact 14-stage completion schema. Empty stage maps cannot complete a run.
- The adapter consumes the unchanged real libtest invocation and prefixed v1 events, bounds output before decoding, rejects duplicate JSON keys and unsupported owner shapes, requires exactly one final plus successful executable/libtest exit, keeps collected provisional, and conservatively bounds final delivery by monotonic elapsed time and unchanged original Engine timestamp where present. The five-second bound is deliberately conservative and may reject a slow valid run; it cannot renew owner authority.
- Ground truth comparison uses actual resource/image/start/running/PID shapes. It qualifies complete instance/PID binding only; it does not prove mount destination to physical UUID mappings.
- Sampling retains process identity and discards samples crossing exit. Main-process FD/RSS/PSS/I/O remains labeled separately from whole-cgroup memory. Missing cgroup/process observations and unresolved closure are reported as ambiguity, not proof. No combined actor or aggregate task/FD headroom claim is introduced.

Reviewed source hashes:

```text
e7916633559e094382029dbc42e2e72cee69893f54d9863b8b133c1433188ecc  tests/qualification/rw040-combined/run_guest.py
b9f666dd4cc1deb63c07cda1c1d26e59e305a4862f7af5bb7c3a382d7d295c3a  tests/qualification/rw040-combined/guest/combined_guest.py
ac7469711e25cc82b43cc0e313800c1a38c3f19ca68734810b43bbb91c3aaf90  tests/qualification/rw040-combined/guest/probe_adapter.py
839ca8c8d533620821e642d8b1e0b7d1a5c0149f91fb41a94330d1a24a1beab9  tests/qualification/rw040-combined/test_harness.py
1833ced0f8ff8ea79023c09ce292f142c160e0db4aebc9b1eda09644a46cd6dd  tests/qualification/rw040-combined/README.md
9035ba088ecdaf12150a42f072e218244b8fbbf24ce289366ef46eb6fabedd73  tests/fixtures/rw040-combined/probe-contract.md
```

The actual probe executable, same-source installed packages, compiler/build-image provenance and guest runtime results are still separate required evidence. Source GO does not certify an as-yet-unbuilt binary or an as-yet-unrun guest.
