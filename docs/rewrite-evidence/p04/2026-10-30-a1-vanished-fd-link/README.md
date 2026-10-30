# FD-link lookup fails before positive approval

The fresh diagnostic candidate uses source
`c58833c49049e62fc9bcff010c6f07e37779475e`, schema 12 and disposable packages
`0.4.10+fdowner.1`. Its offline Debian build, current UI build and signed repository
checks passed. All 687 recorded product inputs and installed binary/package
hashes join this source.

The guest passed the prerequisite installed scenarios and all three negative
pre-effect API cases. It created the positive plan, then approval returned HTTP
503 instead of 200. Root records `link.open Unavailable errno=2`, followed by
`fds.link`, `scan.fds` and `scan.recheck.collection`, during host-consumer
revalidation. The FD-link lookup returned `ENOENT`; the affected owner, FD number
and precise lifecycle cause were not retained. No positive mount job or
journal-held execution was reached.

The new membership event emitted no record because the link failure returned
before the after-list comparison. This is a coverage gap in diagnostics, not
evidence that the FD table was stable. The source already had the owner-before
identity and enumerated FD number, so the next diagnostic must attach those
facts to per-link errors as well. Keep the existing underlying errno record;
do not infer a new errno or query the failed owner afterward.

The detached guard completed from `2026-10-30T10:47:22.950288+00:00` to
`2026-10-30T10:51:33.590137+00:00`, exit 1, with unchanged inputs and no sampled
memory stop. Minimum host availability was **5,109,948,416 bytes**, above the
unchanged 3-GiB floor. The guard was observed live and absent after terminal
completion. Disposable VM scratch was removed. No failed flow was retried and
no approved mount, verification or replay/restart acceptance is claimed.

`source-match.json` holds the concise result. `a1-failure.json.gz` retains the
full bounded guest packet. Raw build/guard output, package/fixture inputs,
launch records and startup diagnostics are retained. Exact guest, runner,
harness and relevant Rust sources are in `reproduction-sources.tar.gz`.
Private keys, binaries, packages and VM disks are excluded. `SHA256SUMS` covers
every other packet file.

The journal reservation and membership diagnostics retain their separately
qualified native scope. Extend owner context to the earlier failure and use a
distinct diagnostic candidate; every predicate, original lifetime and ceiling
remains required. Installed UI and wider A/B/cutover gates stay open. Holly's
production Pi was untouched and nothing was pushed.
