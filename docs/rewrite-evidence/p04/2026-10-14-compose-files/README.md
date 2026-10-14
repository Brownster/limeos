# Protected Compose files: local acceptance

Qualified runtime `7359a97756c898aa0cc324c905cca9c9fedf4851`, based on local main `19beb67`.
The unregistered fixed-bound reader retains protected root/ancestor/file facts
for one recognized operator file and an explicitly selected fixed managed file.
See the [design](../../../plans/2026-10-14-compose-file-evidence-design.md)
and [interface guide](../../../p04-compose-file-evidence.md).

**759 Rust cases pass:** 732 unit/integration cases and 27 compile-fail doctests.
The 27 new cases and focused rerun are already in that total. Full workspace
checks enable the existing opt-in local worker fixture feature.
**61 local ARM64 harness tests pass**, plus strict feature all-target Clippy,
fmt, contracts and repository checks. [checks.json](checks.json) binds commands,
environments, source hashes and complete lossless merged stdout/stderr.
Initial/reviewed precommit checks also pass; their logs and dirty-source hashes
remain as diagnostics rather than exact-source acceptance. The last source review
adds a late post-content directory/absence regression. Documentation edits after
qualification change no runtime source.

24 owned-descriptor cases cover all four operator basenames and optional managed
selection, formatting/anchor preservation, arbitrary binary/empty contents and
secret omission, alternate files/dangling links/FIFOs/directories, mandatory file
and explicit managed presence, selected symlink/FIFO/directory/hardlink refusal,
root/ancestor/file modes and ownership, changed names/content/metadata/ancestors,
unchanged unrelated entries and late file/directory/absence changes after content
checks. Three compile-fail cases prevent importing selection/report/owner types.

Original wall/monotonic age, cooperative work deadline, cancelled chunked reads,
reentrancy/concurrent refusal/unwinding and poisoned retries pass. Every failed
validation invalidates the owner. Exact canonical derived path/component, file,
retained-descriptor, requested-byte and encoded-report bounds pass. Successful
checks preserve the original lease and spend the original ledger. A real nsfs
primitive test compares owned current-namespace references. Public nonroot refusal
precedes callback, namespace and target access.

Successful files use explicitly private scratch-root/UID/clock substitutions
through production traversal/hash code. The production maximum is 19 retained
descriptors, including a namespace reference; the maximum-depth scratch case
measures 18 without that reference. Six additional validation descriptors is a
conservative code bound, not an installed whole-worker observation. Public root-
positive/foreign namespace/PID-1 mount-view qualification stays open. No capability
or confinement guard is relaxed.

SHA256 streaming uses a fixed 4 KiB stack buffer and reports raw hashes/metadata,
never full bodies or environment. One operator and at most one managed file each
fit 64 KiB. The original four-second lease, two-second per-pass cooperative window,
1 MiB lifetime requested-read ledger and 16 KiB compact report remain fixed.
Sentinels and repeated fresh/held comparisons spend the ledger before I/O, with
no refund or renewal. Separate files and checks are no writer transaction. Kernel
calls still require independent supervision; peak allocator/RSS/cgroup headroom
is unqualified. The existing combined worker/probe/resource model is unchanged.

Actual files do not establish approved catalog digests, management identity,
genuine Engine-to-file binding or stack claim keys. YAML/.env/includes/interpolation,
combined Compose semantics, custom overrides, empty-stack discovery, complete
consumers, claims/locks/approvals/effects/recovery and independent review remain
open. The implementing agent reviewed this slice; independent review is pending.
Root/installed confinement, performance and cutover remain open. The frozen
signed ARM64 candidate excludes it. Existing SHA256/hex pins are reused directly,
without dependency version or registry download changes.

Build/cache/new file fixtures and general scratch stayed on disk. Existing storage
fixtures compiled there before small owned auto-cleaned tmpfs runtime scratch.
50 ms samples observed lower-bound peaks of 11579 bytes/7 files,
not exact maxima. No Pi, SSH, real Docker, raw block probe, guest, reserved backup
or external qualification path was used. Service guards remain 256 FDs, 64 MiB,
zero swap and 16 tasks. Historical inventory p95 61.401 ms exceeds 20 ms; no P04,
defect, footprint, performance or cutover gate closes.

[source-manifest.json](source-manifest.json) binds 330 source, fixture,
contract and script files. Fixed-timestamp gzip preserves all log bytes with raw/
compressed hashes. [artifact-sha256.json](artifact-sha256.json) binds the packet.
Reproduce with fresh disk logs using retained helpers; compile existing storage
fixtures before their small owned tmpfs runtime run.

Next is approved catalog/hash/management binding to the retained actual files and
genuine Engine membership. This slice supplies file facts, not stack ownership.
