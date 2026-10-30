# Context on FD-scan failures before membership comparison

Source `c9ddcfc` attaches already-read owner-before identity and the optional
enumerated FD number to directory, before/after enumeration, per-FD deadline
and link failures. It covers the earlier return seen in the
[installed `ENOENT` run](../2026-10-30-a1-vanished-fd-link/README.md).

The wrapper forwards each original failure unchanged. Its context record keeps
`errno: null`: only the existing leaf I/O diagnostic has the actual errno.
Unknown FD or namespace data also remains explicitly absent. No later proc
lookup, new read, retry, filter, predicate exception or lifetime renewal is added.
Owner-before facts still precede the task's later identity revalidation.

Final-source checks passed:

- All **18** focused host-consumer cases, including a real missing proc-FD
  lookup, preserved error classes and maximum-sized/unknown context fields.
- Full storage runtime: **235 passed**, with the ignored isolated helper
  explicitly invoked by its parent regression.
- 13 storage compile-fail doctests.
- Strict storage/executor all-target Clippy, Rust formatting and repository checks.

The full runtime used the exact compiled executable SHA-256
`96f4a85632ca0e51fafc8dddc78d53e58212b3e4fbb32df9adde5e42e0a89756`.
Only fixture data used the established private tmpfs runner: sampled peak
**20,480 bytes**, 16-MiB sampling threshold and 16-MiB per-file limit. Scratch was
removed. Compilation, caches, output and evidence remained on repository disk.

All **687** recorded product inputs match this commit. Checks ran on those bytes
before commit, retaining parent HEAD `ade6a2d` and their original timestamps.
Every phase has its own successful original terminal guard record, unchanged
inputs and no sampled 3-GiB memory stop. The sequential driver ran in a detached
process session, with signal-aware guard wrappers, so loss of a tool connection
could not remove supervision. Launch-record helper hashes match the archived
scripts. The separate formatter procedure intentionally changed inputs and
supplies no qualification; its final hashes match subsequent checks.

`source-match.json` joins all checks, runtime identity and resource observations.
Raw output/records, compile JSON, runtime data and qualification launch/console
records are retained. Exact Rust source, guard, wrapper, runner, sequential
driver, launcher and archiver are in `reproduction-sources.tar.gz`.
`SHA256SUMS` covers every other packet file.

A fresh diagnostic candidate must still identify the actual failed owner and
FD before any runtime correction is selected. No installed mount, independent
verification, replay/restart, UI or broader A/B/cutover acceptance is claimed.
Holly's production Pi was untouched and nothing was pushed.
