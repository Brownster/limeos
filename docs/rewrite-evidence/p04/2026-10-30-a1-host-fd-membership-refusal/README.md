# Host FD-table change before positive plan creation

The fresh candidate uses source
`e19fca92c0072cd2d2ddb241a2d10c8c9564e8f4`, schema 12 and disposable standard/shadow
packages `0.4.9+journal.1`. Its offline Debian build, current UI build and signed
repository checks passed. All 687 recorded product inputs and the installed
binary/package hashes join this source.

The 384-MiB guest passed the preceding installed storage, claims, target-job
recovery and package scenarios, then all three negative pre-effect API cases.
The next positive plan request returned HTTP 409 instead of 201. No positive
mount job was created; this guest never exercised the new journal-held execution
profile. Its native qualification remains [separate evidence](../2026-10-30-a1-typed-journal-reservation/README.md).

Root records `scan.fds Conflict`, followed by `scan.recheck.collection`,
`recheck.host_consumers.before` and `target.final_recheck`. Source tracing finds
the remaining unlabelled `Conflict` in `check_fds`: the sorted FD-number lists
before and after link checks differ. The failed owner and compared FD numbers
were not retained. Existing task/census and capacity diagnostics emitted no
records. Later process snapshots cannot establish which FD table changed or why.

The original detached guard completed from
`2026-10-30T00:35:41.007619+00:00` to `2026-10-30T00:39:39.081001+00:00`, exit 1,
with unchanged inputs and no sampled host memory stop. Minimum availability
was **6,749,884,416 bytes**, above the unchanged 3-GiB floor. The recorded guard
was observed live during the run and absent after completion; VM scratch was
removed. No failed flow was replayed and no mount acceptance is claimed.

`source-match.json` contains the concise result and joins. `a1-failure.json.gz`
retains the bounded full guest packet. Raw build/guard output, package/fixture
inputs, launch records and startup diagnostics are retained. The 31 guest
modules, runner support, exact harness and relevant Rust source are compacted
in `reproduction-sources.tar.gz`. Private keys, binaries, packages and VM disks
are excluded. `SHA256SUMS` covers every other packet file.

Next, add failure-only diagnostics for the exact FD-number comparison with its
already-read owner-before identity. Keep every equality check, deadline and
resource ceiling; do not infer harmless churn, filter a process or retry the
failed flow. Genuine mount, independent verification, replay/restart, installed
UI and the wider A/B/cutover gates remain open. Holly's Pi was untouched and
nothing was pushed.
