# Reservation clock admission, 21 October 2026

Source `7f22d135fdac9a1f4aa663ec2b5cf4229aeacfd4` implements
[post-lock reservation admission](../../../p04-b1-reservation-clock-handoff.md).
The trusted clock is sampled inside the existing Store write transaction; the
original monotonic deadline is retained across lock waits and checked before
commit. The existing deterministic fixture entry and SQL invariants remain.

| Check | Result |
| --- | --- |
| Focused reservation/recovery module | 22 passed, overlapping the full unit run |
| Entire persistence crate | 133 unit tests and two compile-fail doctests passed |
| Strict all-target persistence Clippy | Passed with `-D warnings` |
| Workspace Rust formatting check | Passed |
| Source identity | All 677 tracked inputs in each successful snapshot match the commit |

Four new test functions exercise real SQLite lock waits, a changed due slot,
a closed UTC window, original deadline expiry, successful fresh-time job
deadlines, restart replay, changed owner revision, expiry inside the clock
callback before an insert trap, and invalid/expired admission. Existing rollback,
capacity, authority and startup-corruption tests remain enabled.

`source-match.json` binds the successful commands and results to the commit.
Compressed records retain full before/after input hashes, sampled host memory
and raw output. `source.patch.gz` retains the implementation commit. `SHA256SUMS`
covers every other file in this packet.

The initial regression command was refused before compilation below the 3-GiB
floor and has no test assertion result. The first candidate test build was
terminated by that unchanged host guard before tests; its failed record/output
are retained. After increased headroom, the candidate passed the focused and full
runs. Two earlier formatting commands succeeded while intentionally modifying
their inputs; those mutation runs are not qualification records.

These are local native persistence checks. No timer, producer, observation
completion, alert/outbox migration or delivery worker is registered. No installed,
package, ARM64, Pi, idle-write or complete B1 acceptance is claimed. Commit/fsync
completion is not guaranteed to precede the operation deadline. Full cutover
still requires the remaining P04 runtime and P05/P06 gates.
