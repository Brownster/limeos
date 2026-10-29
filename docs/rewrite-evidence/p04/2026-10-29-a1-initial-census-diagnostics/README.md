# Initial host-consumer census diagnostics

Source `9a7d6fd05b70066fd58b6020d8dfe488525b379e` labels the four previously
unlabelled comparison groups in the initial host-consumer scan. Failure-only
events retain the already compared numeric identities, counts and at most four
added/removed IDs per side. Events stay below 2 KiB at maximum tested inputs.
Unknown namespace data is explicitly absent. Read order, short-circuiting,
`Conflict` results and the existing admission budgets remain intact.

The source join verifies all **687** recorded product inputs against this commit.
Most checks ran against its identical uncommitted bytes on parent `c1a4dd6`;
their original HEADs and timestamps remain in the records. The repository check
ran after the source commit. This packet qualifies diagnostics; it supplies no
installed approved-mount result.

| Check | Result |
|---|---|
| Focused host-consumer tests | 15 passed |
| Direct full storage run, repository Btrfs scratch | 214 passed, **17 failed** |
| Exact compiled storage test binary, private tmpfs fixtures | 231 passed |
| Storage compile-fail doctests | 13 passed |
| Strict storage and executor all-target Clippy | Passed |
| Rust formatting and repository hygiene | Passed |

The disk-based failures include a directory-link-count assumption (`1 == 1`)
and 16 source-fixture refusals. Their full failure output is retained. A separate
no-run Cargo record identifies the executable used by the existing fixture-only
runtime runner. Its SHA-256 is
`bc074ad499e7d132900798ce2595e67d35778b26ff1a588c57df6c20d02dc386`.
The same executable passed all 231 tests using a private directory under
`/run/user/1000`. This supports a fixture filesystem dependency; it does not
qualify the failed Btrfs run or establish installed mount behavior.

Only runtime fixture files used tmpfs: sampled peak **73,976 bytes**, a 16-MiB
sampling threshold and a 16-MiB per-file limit. The runner removed its scratch
directory. Compilation, caches, logs and evidence stayed on repository disk.
All recorded checks preserved their source inputs and completed without the
sampled 3-GiB host memory guard firing. Memory observations are in each record.

`source-match.json` joins the checks and runtime record. `checks/` retains raw
guard records and output, including the failed disk run and compilation JSON.
`harness/` retains the guard, fixture runner and archiver; `source.rs.gz` retains
the exact diagnostic source. `SHA256SUMS` covers every other packet file.

Next, rebuild a distinct Debian candidate and run a fresh disposable guest to
identify the actual initial-census comparison. Preserve every original
deadline, identity check and resource ceiling. Approved API/UI mount acceptance,
restart/replay, broader A/B runtime qualification and Pi cutover remain open.
No production Pi access or push formed part of these checks.
