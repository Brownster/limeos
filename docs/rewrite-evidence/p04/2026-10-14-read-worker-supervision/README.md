# Read-worker supervision: local acceptance

Qualified runtime `25c92bab2dc8c20c948b90a4327510eb3447295c`, based on local main `e1faf39`.
Unregistered manager/guardian mechanics independently supervise owned worker
processes. Admission stays occupied through confirmed cleanup; unknown completion
poisons it. A dedicated session/subreaper guardian observes manager pipe EOF and
the original deadline, signals only pinned child handles and reaps all children,
including adopted descendants in other sessions. Worker bootstrap establishes
parent-death signaling and inherited NOFILE=226 before genuine collection.
See the [design](../../../plans/2026-10-13-read-worker-supervision-design.md) and
[interface guide](../../../p04-read-worker-supervision.md).

**675 Rust cases pass:** 656 unit/integration cases and 19 compile-fail doctests.
The 17 new cases and focused rerun are already in that total. Workspace tests use
the opt-in `limeos-executor/read-worker-probe` feature; the fixture binary is
disabled by default and absent from the packaging executable list.
**61 local ARM64 harness tests pass**, plus strict all-target Clippy, fmt,
contract and repository checks. [checks.json](checks.json) binds final-source
commands, environments, runtime hashes and lossless transcripts. Documentation
edits during validation changed no runtime source.

Ten owned-process tests cover actual deadline termination of synchronous sleep,
cancellation/drop, contention, manager SIGKILL with orphan observer reaping,
escaped-session child cleanup, inherited output-pipe cleanup, complete JSON/frame
and successful exit requirements, malformed/partial/trailing/oversized output,
nonzero exit and guardian-failure poisoning. Both small and full 256 KiB reports
pass. Marker checks confirm recorded fixture PIDs have gone after cleanup. The
parent's descriptor limit stays unchanged. Bootstrap plus the genuine public
collector passes resource admission and refuses nonroot configuration access;
this qualifies no root-positive storage/Engine composition.

Private unit tests cover complete/escaped JSON bounds, missing EOF, restricted
status tags, buffer capacity, late/cancelled queued reports and poisoning before
permit drop. Compile-fail coverage prevents importing report bytes as authority.
[Diagnostics](diagnostics/README.md) retain the cleanup selector failure: the
original waitpid(None) overlooked children outside the guardian's group. The
correct all-child wait and pinned-handle cleanup pass the retained regressions.
The implementing agent reviewed this slice; independent review remains pending.

The ticket deadline can return while cleanup or launch remains pending, holding
admission. This is no guarantee of termination for blocked exec or an unkillable
kernel task, nor a strict scheduler-latency bound. Guardian SIGKILL cannot confirm
descendant cleanup; future service cgroup teardown remains the final guard.
Arbitrary commands/frame JSON do not authenticate storage facts: protected fixed
production launch/configuration selection is not integrated. No production
command/RPC/route/schema/service is added; the combined qualification probe stays
blocked. Installed capabilities, namespaces and syscall compatibility remain open.

Service guards remain 256 descriptors, 64 MiB, zero swap and 16 tasks. Whole
manager/guardian/worker peak resource headroom is unqualified. Historical inventory
p95 61.401 ms remains above the 20 ms target. No footprint or performance gate closes.
The frozen signed ARM64 candidate excludes this slice.

Build/cache/general scratch stayed on disk. Existing storage fixtures were compiled
there before owned, auto-cleaned tmpfs runtime scratch. Their 50 ms samples observed
lower-bound peaks of 69045 bytes and 4 files, not exact maxima.
No Pi, SSH, Docker, raw block probe or guest launch was used. Reserved backup work
and external qualification paths stayed untouched.

[source-manifest.json](source-manifest.json) binds 316 source,
fixture, contract and script files. Fixed-timestamp gzip preserves complete merged
stdout/stderr bytes with raw/compressed hashes. [artifact-sha256.json](artifact-sha256.json)
binds the packet. Reproduce with fresh disk logs using the retained helpers;
compile storage fixtures before their tmpfs runtime run.

Next is protected fixed launch/configuration selection and the test-only combined
qualification seam, followed by installed root/capability/namespace and resource
qualification. Complete physical consumers/claims, actual pool/share state, locks,
effects and recovery remain open. P04, defect and cutover gates stay open.
