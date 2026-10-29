# Genuine mount-journal descriptor footprint

Source `d65fb71` adds an isolated native resource regression for the genuine
`MountJournal::from_directory` constructor using its existing private owned
directory fixture. Production admission, journal policy and execution remain
unchanged. Fixtures and output stay on repository disk; no mount or production
path is used.

| Observation | Open descriptors |
|---|---:|
| Isolated process baseline, including observation directory | 4 |
| First journal creation | 9 |
| After a durable receipt write | 9 |
| After dropping that journal | 4 |
| Reopening the same journal | 9 |
| After dropping the reopened journal | 4 |

The real owner retains **five** descriptors: protected directory, process lock,
SQLite database, WAL and shared-memory file. The initial diagnostic retained
the raw numeric record. The final regression asserts the five-descriptor delta
through create/write/reopen and complete release on drop. It counts native
directory entries with explicit I/O-error propagation. A parent regression
executes the ignored helper in its own test process and requires exactly one
result marker, so an empty filtered child run cannot pass.

All **14 journal cases passed**, including the new parent regression; its ignored
child helper ran within that regression. Strict storage all-target Clippy, Rust
formatting and repository checks passed. The rest of the kernel fixture suite
was unchanged and was not repeated for this test-only addition.

All **687** final recorded product inputs match this commit. Checks retain their
original parent HEAD `1a0ac5b` and timestamps. The earlier measurement and failed
initial formatting check use preserved exact variants of `effects/tests.rs`;
every other recorded product input is identical. Both variants and their raw
records are retained. Every run completed with unchanged inputs and without the
sampled 3-GiB host memory stop firing. `source-match.json` records the final test
executable hash, all checks and the measured numeric record. `SHA256SUMS` covers
every other packet file.

This confirms a real execution-only descriptor footprint. It does **not** reveal
the unlogged exact sample in the [installed capacity refusal](../2026-10-29-a1-execute-capacity-refusal/README.md).
Next, give the genuine retained journal an explicit typed reservation within
the unchanged aggregate model, preserving ordinary inspection limits and all
replay, locking, expiry and uncertain-effect rules. A fresh installed candidate
must still prove mount, independent verification and restart/replay; installed
UI and wider A/B/cutover gates remain open. Holly's Pi was untouched and nothing
was pushed.
