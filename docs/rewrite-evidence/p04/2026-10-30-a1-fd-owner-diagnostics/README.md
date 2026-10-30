# Bounded FD-table owner diagnostics

Source `6ed0155` labels the exact sorted FD-number comparison that refused the
[previous installed plan](../2026-10-30-a1-host-fd-membership-refusal/README.md).
The failure-only `fds.ids_changed` collection event retains the already-read
`owner_before` PID/TID, start ticks and known mount namespace, full difference
counts and at most four added/removed FD numbers per side.

The label describes the identity observed before this comparison. An early
refusal precedes later task revalidation; it supplies no reconstructed final
owner, open-file identity or harmlessness classification. No extra proc reads,
paths, command lines, environments or FD-link text are logged. Exact equality,
read order, `Conflict`, original lifetime, poisoning and every consumer predicate
remain intact. Maximum-input events are tested below 2 KiB.

Final-source checks passed:

- All 16 focused host-consumer cases, including same-count replacement,
  ordering refusal, exact owner-before facts and bounded numeric samples.
- Full storage runtime: **233 passed**, with the isolated helper invoked by its
  parent regression and ignored by the outer harness.
- 13 storage compile-fail doctests.
- Strict storage/executor all-target Clippy, Rust formatting and repository checks.

Cargo compiled the exact test executable on repository disk. The separate
runtime runner joined that compile record and executable SHA-256
`35c857dbca2e6afff55c7af66475dc148d9f77fcd930000245d89cc29ffd2560`.
Only fixture data used the established private tmpfs directory: sampled peak
**65,537 bytes**, 16-MiB sampling threshold and 16-MiB per-file limit. The runner
removed its scratch. Compilation, caches, logs and evidence stayed on disk.

All **687** recorded product inputs match this source commit. Checks ran against
those bytes before the commit, with parent HEAD `c28d23b` and original timestamps
retained. Every check preserved its inputs and completed without the sampled
3-GiB host memory stop. The formatter procedure intentionally changed inputs
and supplies no qualification; its final hashes match subsequent checks.

`source-match.json` joins the checks and runtime identity. Raw output, compile
JSON, guard records and runtime observations are retained. Exact Rust source,
guard, fixture runner, sequential check driver and archiver are compacted in
`reproduction-sources.tar.gz`. `SHA256SUMS` covers every other packet file.

Build a distinct diagnostic candidate to identify an actual changing owner and
FD comparison before choosing a runtime correction. This packet qualifies the
diagnostics, not an installed mount. Genuine mount, independent verification,
replay/restart, installed UI and wider A/B/cutover gates remain open. Holly's Pi
was untouched and nothing was pushed.
