# Declared storage impact: local acceptance

Qualified runtime `4e7f73d71f8cdf84e14489277fa371941bb05332`, based on local main `be705f2`.
The pure unregistered planner validates full expected-device selections against
all original typed configurations and the checked configured graph. It preserves
original Docker age and full relationships, then derives bounded typed resource
candidates and full running container snapshots for review. See the
[design](../../../plans/2026-10-14-storage-impact-preview-design.md) and
[interface guide](../../../p04-storage-impact-planning.md).

**706 Rust cases pass:** 684 unit/integration cases and 22 compile-fail doctests.
The 16 new cases and 15-case focused rerun are already in that total.
Full workspace checks enable the existing opt-in local worker fixture feature.
**61 local ARM64 harness tests pass**, plus strict all-target Clippy, fmt,
contract and repository checks. [checks.json](checks.json) binds commands,
environments, runtime hashes and full merged stdout/stderr logs. Documentation
edits during validation changed no runtime source.

Four independent frozen-layout goldens cover selected/related devices, resource
subjects and running dependents. Tests preserve the full checked graph, original
time, complete expected metadata, numeric ownership, branch order, share settings,
source/destination/volume details and flags. Unknown, duplicate, missing, excessive
or changed targets refuse with sanitized fixed-field findings. Invalid unused
configuration and stale/future original Docker declarations also refuse.

Affected pools include all configured member device/mount candidates. Any affected
SnapRAID drive includes one global protection subject and all protection members.
Related members do not become unmount targets or import unrelated direct consumers
and other pools into selected-device dependents. Selecting those devices explicitly
does include their consumers. Stopped/read-only containers and disabled/read-only
configuration remain candidate resources; running snapshots deduplicate full IDs.
Literal case/component boundaries, ancestor sources, named volumes, ordering and
input immutability pass. No stack ownership is invented from container IDs.

Exactly 128 distinct subjects pass; the 129th refuses the whole preview. The graph's
existing global 512-route ceiling remains enforced before selection. A valid
490-route graph with long paths exceeds 256 KiB and refuses without a partial
preview. Borrowed compact JSON serialization counts bytes without a second report buffer.
The bounded typed graph is constructed before that check; peak allocation and
installed service headroom remain unqualified. The existing workspace serde_json
pin moves to domain runtime use for counting; no dependency version changes.
Compile-fail coverage prevents importing the serialize-only result.

Every result keeps eight missing evidence boundaries explicit: retained identity/
freshness, complete physical consumers, actual pool/protection/share state, stack
ownership, complete claims/approval and effect-time validation. Empty declared rows
and unmapped shares cannot prove safe unmounting. Typed resource subjects are
independent of Store keys and acquire no claims. [Diagnostics](diagnostics/README.md)
retain the first invalid pool fixture and its correction under unchanged frozen
rules. The implementing agent reviewed the slice; independent review is pending.

No combined collector/report, Store claim path, schema, route, command, service
guard or package executable list changes. Domain bounds do not qualify the smaller
CombinedV1 worker counts. Root/foreign-namespace and installed qualification,
complete physical consumers/claims, actual pool/share state, locks/effects/recovery,
resource headroom and cutover remain open. The frozen signed ARM64 candidate
excludes this slice. Service guards stay 256 FDs, 64 MiB, zero swap and 16 tasks.
Historical inventory p95 61.401 ms remains above the 20 ms target. No P04, defect,
footprint or performance gate closes.

Build/cache/general scratch stayed on disk. Existing storage fixtures were compiled
there before small owned, auto-cleaned tmpfs runtime scratch; 50 ms sampling
observed lower-bound peaks of 6010 bytes and 4 files, not exact
maxima. No Pi, SSH, Docker, raw block probe or guest launch was used. Reserved
backup work and external qualification paths stayed untouched.

[source-manifest.json](source-manifest.json) binds 323 source, fixture,
contract and script files. Fixed-timestamp gzip retains full log bytes with raw/
compressed hashes. [artifact-sha256.json](artifact-sha256.json) binds the packet.
Reproduce with fresh disk logs using the retained helpers; compile storage
fixtures before their small tmpfs runtime run.

Next local integration requires protected physical closure, actual runtime state
and stack identity before candidates can bind complete claims. Installed/root
qualification of the supplied combined probe remains in the separate workstream.
