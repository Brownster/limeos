# Bound Docker fixture startup before the first plan

Fixture source `658ccdb2e7fb2d3c9fb9c6d84d5d71bb3e162d2d` adds a
read-only startup probe for the real disposable Docker daemon. It exercises
`/version`, versioned `/info` and `/containers/json?all=1`, checks the socket
peer, Engine identity and genuine empty inventory, then samples natural task/FD
settling. It requires three quiet seconds within a 15-second deadline and
96-pass cap. Proc membership and response sizes are bounded; identity change,
malformed replies, nonempty inventory and persistent churn fail setup. No
production admission evidence, plan or effect is obtained or reused by this
probe. Every production comparison and ceiling remains unchanged.

All 15 readiness cases and the full **91 reference tests** pass natively.
The first full run hit sandbox `EPERM` in existing Unix socket-pair tests; that
failed record is preserved. Native runs passed, including the final formatted
source. Their precommit record HEAD differs from the fixture commit, but all
687 recorded input hashes match that commit. Scoped Ruff checks, formatting,
repository hygiene and diff checks pass.

The installed run reused immutable packages `0.4.11+fdcontext.1`, built at
`f6f583e69518a732bcb8a34b2d09c7f4e3b7e23d`. All production inputs remain
identical. The manifest explicitly records the three changed test files; only
two of the 31 frozen guest modules changed. The runner is the exact committed
canonical 1024-MiB runner. Package, binary, UI and schema hashes retain their
original build identity; this is not a new product build.

The fresh guest observed three Docker membership changes during 50 read passes,
then 3.0068 quiet seconds, completing setup in 5.9881 seconds. It passed all
three negative API cases, created the genuine positive plan, approved it and
queued the mount. Execution ended with Core `needs_intervention`. No admission
diagnostic event was emitted, and the retained service journal does not explain
the exact Root execution stage. The actual Root receipt and independent later
mount state were not captured. This supplies **no accepted mount result** and
does not establish that the effect did or did not occur. The failed action was
not retried.

The detached guard completed 16:13:03–16:17:14 UTC on 30 October, exit 1,
with unchanged inputs and no sampled memory stop. Minimum host availability was
5,394,800,640 bytes, above the unchanged 3-GiB floor. Its process was observed
absent after completion; disposable VM scratch was removed. No production Pi
access or push occurred.

Capture the bounded actual Root receipt/quiescence and retained Root/Core claims
after failure, before guest disposal. These are later observations, never fresh
execution authority or permission to replay. Use that result to locate the
uncertain stage while retaining original lifetimes and recovery barriers.
Installed UI, accepted mount/replay/restart, ARM64 and broader A/B gates remain
open.

`source-match.json` records the joins and scope. Compressed original guard,
fixture, package, local-check and failure records retain the result. Exact
fixture and launcher sources are in `reproduction-sources.tar.gz`.
`SHA256SUMS` covers every other file; private keys, packages, binaries and VM
disks are excluded. The package build's full reproduction packet remains in
[the preceding candidate evidence](../2026-10-30-a1-docker-startup-and-deadline/README.md).
