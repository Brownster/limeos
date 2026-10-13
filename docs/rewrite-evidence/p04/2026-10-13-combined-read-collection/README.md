# Retained combined collection: local acceptance

Qualified runtime `ed6bd09c936fb281dc36b5e6eaba49c7a46f03e3`, based on local main `56afdd6`.
The unregistered collector calls genuine closed-profile configuration, Engine,
process, physical and destination adapters, retains their owners, and computes
the configured graph. Sequential budget phases precede acquisition. Final
configuration/Engine/destination/Engine/configuration comparisons preserve all
original leases and the Engine lifetime byte ledger. One borrowed 256 KiB body
is exposed only after a final original-window check. See the
[design](../../../plans/2026-10-13-combined-read-collection-design.md) and
[interface guide](../../../p04-combined-read-collection.md).

**658 Rust cases pass:** 640 unit/integration cases and 18 compile-fail doctests.
The 14 new cases are included; focused and measurement reruns add no cases.
**61 local ARM64 harness tests pass**, with strict all-target workspace Clippy,
fmt, contract and repository checks. [checks.json](checks.json) binds exact-source
commands, environments, runtime hashes and lossless transcripts. Documentation
edits during validation changed no runtime source.
The initial sandbox denied unchanged owned Unix-socket fixture binds; the full
approved local-test rerun passed. [Diagnostic evidence](diagnostics/README.md)
retains that failed transcript without adding its counts to acceptance.

Window tests cover exact monotonic expiry, independent wall expiry, rollback
above original issue time, negative/backward samples, permanent refusal, failure
stage/reason, cancellation before polling and after completed work, discarded
publication bytes and unchanged original start time. A real synchronous sleep
blocks the test runtime beyond four seconds; its completed value is refused and
dropped after return. This explicitly demonstrates the need for independent
supervision. The compile-fail check rejects report import as evidence.

An owned test child lowers only its own soft descriptor limit to 226, uses
production own-process admission and calls the genuine public collector as UID
1000. The configuration root guard refuses before Engine/storage acquisition;
the parent's limit stays unchanged. This qualifies public nonroot refusal,
not root-positive combined collection, installed capability behavior, process
termination or cleanup. Private gate/window fixtures do not construct fake public
owners. Existing frozen layouts and closed-profile regressions remain included.
The implementing agent reviewed the code; independent review is pending.

No launcher, command, RPC, route, schema or service is added. The standalone probe's
combined case remains blocked. The four-second window is cooperative and the
comparisons are sequential, not atomic. Resource reservations and sampled counters
prove no peak RSS/cgroup headroom; the unchanged 256-FD/64 MiB/zero-swap/16-task
service restrictions still need installed qualification. Historical inventory
p95 61.401 ms remains above the 20 ms target. No performance gate closes.

Source/build/cache/log/general scratch stayed on disk. Existing storage fixtures
were compiled there before owned, auto-cleaned tmpfs runtime scratch. Their 50 ms
samples observed lower-bound peaks of 53248 bytes and 8
files, not exact maxima. No Pi, SSH, Docker, raw block probe or guest launch was
used. Reserved backup recovery and external qualification paths stayed untouched.

[source-manifest.json](source-manifest.json) binds 311 source,
fixture, contract and script files. Fixed-timestamp gzip preserves every original
stdout/stderr byte with raw/compressed hashes. [artifact-sha256.json](artifact-sha256.json)
binds the packet. Reproduce with fresh disk logs and the retained helpers; compile
storage fixtures before their tmpfs runtime run.

Next is independent worker supervision: single-flight ownership until cleanup,
deadline/cancellation independent of blocked work, parent death, and descendant
termination/reaping. Root/capability/namespace positives, original-age races,
installed footprint/performance, complete physical consumers/claims, actual
pool/share state, locks, effects and recovery remain open. The frozen signed
ARM64 candidate excludes this slice. P04, defect and cutover gates stay open.
