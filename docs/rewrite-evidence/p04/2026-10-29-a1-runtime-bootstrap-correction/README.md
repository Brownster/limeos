# Mount-executor runtime bootstrap correction

Source `5945dfb` removes the no-op blocking worker created at mount-service
startup. Tokio's later idle retirement changed the executor's own host-task
membership in the [installed diagnostic](../2026-10-29-a1-executor-idle-thread/README.md).
The current-thread runtime, one-thread blocking-pool ceiling for storage roles,
early guardian bootstrap and every host-consumer comparison remain intact.

The new native regression launches an isolated test process through the private
runtime bootstrap used by `main`. It compares actual `/proc/self/task` identities
before bootstrap, inside its future and after runtime shutdown. It also exercises
real Tokio Unix-stream I/O and owned child-process output/waiting.

With the original startup spawn retained, the regression failed because bootstrap
added a third thread to the two-thread isolated test process. Removing that spawn
made the same regression pass. Its nested helper is deliberately ignored by the
outer test harness and explicitly invoked by the parent regression; a required
success marker prevents an empty filtered child run from passing.

| Final formatted source check | Result |
|---|---|
| Executor library tests | 48 passed |
| Executor binary tests | 2 passed, isolated helper invoked within the regression |
| Default executor integration cases | 5 passed |
| Executor compile-fail doctests | 6 passed |
| Strict all-target executor Clippy | Passed |
| Rust formatting and repository hygiene | Passed |

The default test run does not enable the separate `read-worker-probe` feature;
zero-case gated integration executables supply no feature acceptance. No new
installed mount, UI, ARM64, upgrade or full resource acceptance is claimed here.

All **687** final recorded product inputs match the committed source. The checks
ran on those bytes before the source commit, with original parent HEAD `5723452`
retained in each record. The red variant differs only in `main.rs`; the first
green variant and the failed initial formatting check use an exact reconstructed
pre-format version of that same file. `source-match.json` verifies all variants
by their actual hashes. Both failed records are preserved unchanged.

Every recorded run preserved its inputs and completed without the sampled
3-GiB memory stop. Compiler caches, scratch, output and evidence remained on
repository disk. `harness/` retains the exact guard, final sequential check
driver and source-joining archiver. `SHA256SUMS` covers every other packet file.

Next, build a distinct source-bound candidate and qualify actual approved mount,
independent verification and restart/replay in a fresh disposable guest under
continuous supervision. The wider A/B, P05 assistant and P06 migration/return
gates remain open. Holly's production Pi was untouched and nothing was pushed.
