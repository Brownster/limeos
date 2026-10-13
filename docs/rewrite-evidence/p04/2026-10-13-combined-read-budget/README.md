# Combined reader budget: local acceptance

Qualified source `94a2826861c94763306e91b23cf17642d5f3acf6`, based on local main `65f1f04`.
The closed configuration profile preflights protected raw JSON shape before
materializing typed values and uses the smaller host parser through revalidation.
The new unregistered bin library supplies resource reservations, fresh worker
samples, lexical sequential phase guards, a single-use bounded report and
nonblocking single-flight ownership. It neither launches nor collects owners.
See [the design](../../../plans/2026-10-13-combined-read-budget-design.md)
and [interface guide](../../../p04-combined-read-budget.md).

**644 Rust cases pass:** 627 unit/integration cases and 17 compile-fail doctests.
The 21 new cases are already included; focused/measurement reruns add no cases.
**61 local ARM64 harness tests pass**, with strict all-target workspace Clippy,
fmt, contract and repository checks. [checks.json](checks.json) binds final-source
commands, environments, exact runtime hashes and lossless transcript hashes.

All four frozen layouts pass the closed configuration profile in the private
scratch root/UID seam. It preserves original expiry, retained handles and typed
contracts. Shape tests cover exact/excess collections, rejection before decoding
an excess malformed token, shared nodes/strings, depth and trailing syntax.
The pinned known configuration cell-size audit passes. Existing Standard and
storage/process/destination regressions remain in the workspace total.

The owned test child lowers only its own soft descriptor limit to 226 and uses
production own-process counter reads. Its parent's limit is verified unchanged.
Budget phases reserve capacity without collecting any Engine/storage/process
owner. Synthetic samples cover exact/excess resources, phase failure/poisoning,
contention and escaped report bounds. Compile-fail checks cover closed-profile
import, session/phase borrowing and single-use reports. This qualifies no public
root authentication, combined collection, independent termination or installed
resource behavior. The implementing agent reviewed this slice; independent
review remains pending.

The plan reserves 178 retained descriptors, 16 baseline and 32 transient, totaling
226 below the service's unchanged 256. The conditional payload model is 21.25 MiB
within 24 MiB, alongside a 24 MiB baseline and 48 MiB sampled-RSS refusal threshold.
The 64 MiB, zero-swap and 16-task service guards stay intact. These arithmetic and
sample checks prove no total heap, peak RSS, cgroup headroom or footprint gate;
exclusions are explicit in [scope.json](scope.json). Single-flight drop is no
child cleanup proof. Actual supervisor ownership and process reaping remain open.

Source/build/cache/log/general scratch stayed on disk. Existing storage fixtures
were compiled on disk before owned, auto-cleaned tmpfs runtime scratch. The full
storage 50 ms samples observed lower-bound peaks of 75055 bytes and
8 files, not exact maxima. No Pi, Docker or raw block probe was used;
reserved backup recovery and external qualification paths stayed untouched.

[source-manifest.json](source-manifest.json) binds 309 source,
fixture, contract and script files. Fixed-timestamp gzip preserves every original
stdout/stderr byte; both hashes are retained. [artifact-sha256.json](artifact-sha256.json)
binds the packet. Reproduce with fresh disk logs and the retained helper; compile
storage fixtures before their tmpfs runtime run.

Next is genuine owner composition in an independently supervised worker,
original-age validation, cancellation, parent death and descendant reaping, then
installed resource qualification. Complete physical consumers/claims, actual
pool/share state, locks, effects and recovery remain open. The frozen signed
ARM64 candidate excludes this slice. P04, defect, performance and cutover gates
stay open.
