# Retained Compose membership: local acceptance

Qualified runtime `a17d1f3bdd91b9cda72c8d0ff0b4f9c37bab15cf`, based on local main `4119b3c`.
The unregistered fixed-endpoint UID-0 reader projects complete Engine-declared
project/service/one-off membership through the genuine authenticated collector.
It retains the same original socket, storage/PID facts, lifetime and spent ledgers.
See the [design](../../../plans/2026-10-14-compose-membership-evidence-design.md)
and [interface guide](../../../p04-compose-membership-evidence.md).

**732 Rust cases pass:** 708 unit/integration cases and 24 compile-fail doctests.
The 26 new cases and focused rerun are already in that total. Full workspace
checks enable the existing opt-in local worker fixture feature.
**61 local ARM64 harness tests pass**, plus strict all-target Clippy, fmt,
contract and repository checks. [checks.json](checks.json) binds commands,
environments, runtime hashes and lossless merged stdout/stderr. Documentation
edits during validation changed no runtime source.

Nine private metadata/validation-state cases cover explicit non-Compose labels,
required fields, exact name/path/file limits, meaningful file order, project-name
ambiguity, full container identity, secret omission, dropped/failed validation
and reentrancy. Two decoder cases prove nested/escaped duplicate-key refusal in
Compose mode while preserving old behavior and allowing equal keys in distinct
objects. Two compile-fail cases prevent importing snapshots or opaque owners.

Thirteen async collection/refusal cases use owned listeners for complete running/stopped/mountless/
non-Compose membership, replicas and one-offs, project-directory/file ambiguity,
malformed/partial metadata, initial-pass and fresh selected-label changes, base
PID/mount/identity/membership/Engine changes, original wall/monotonic expiry,
connection closure on dropped work and permanent invalidation through either
owner accessor. Concurrent validations both refuse; a later success cannot undo
the refusal. GET-only requests and secret omission pass. Real public nonroot
refusal occurs before canonical endpoint access. Root-policy and wrong-kernel-peer
negative fixtures send no HTTP before authorization.

Successful fixture collection uses explicitly private UID/clock substitutions.
It qualifies no public UID-0/installed Engine, independent daemon PID or host
namespace. Project/service/path conventions follow pinned Docker Compose v2.29.7
references linked in the guide; no installed-version compatibility is claimed.
Config-file names are comma-delimited declarations, not trusted argv or files.
Unknown labels/environment are omitted. Hook/relay, partial/malformed metadata
and conflicting selections refuse instead of becoming unmanaged or empty facts.

The existing 16-complete/eight-running/32-selected-mount, JSON shape, 2 MiB
lifetime HTTP and 2 MiB counted live-payload ceilings remain closed. Base and
Compose compact reports together fit the unchanged 48 KiB limit; long valid
metadata plus otherwise qualifying storage facts refuses the total. Revalidation
never replenishes spent input bytes. String/vector/comparison overlap is charged;
projection data drops before its reservation, and final drop returns the ledger.
This is no peak allocator/RSS/cgroup proof or support for the Pi's larger running
count. No service ceiling is raised.

Existing advisory/storage interfaces, snapshots and JSON semantics stay unchanged.
The new reader remains unregistered and outside the combined worker/probe. Labels
do not establish actual protected stack files, management identity, Store claim
keys, complete physical consumers or lifecycle authority. Those prerequisites,
claims/locks/approvals/effects/recovery, independent review, public root/installed
qualification and cutover remain open. Synchronous kernel work still needs
independent supervision. The implementing agent reviewed this slice; independent
review is pending. The frozen signed ARM64 candidate excludes it.

Build/cache/general scratch stayed on disk. Existing storage fixtures were compiled
there before small owned, auto-cleaned tmpfs runtime scratch. Their 50 ms samples
observed lower-bound peaks of 57496 bytes and 7 files, not exact
maxima. No Pi, SSH, real Docker, raw block probe or guest launch was used. Reserved
backup and external qualification paths stayed untouched. Service guards remain
256 FDs, 64 MiB, zero swap and 16 tasks. Historical inventory p95 61.401 ms remains
above 20 ms; no P04, defect, footprint, performance or cutover gate closes.

[source-manifest.json](source-manifest.json) binds 326 source, fixture,
contract and script files. Fixed-timestamp gzip preserves full log bytes with raw/
compressed hashes. [artifact-sha256.json](artifact-sha256.json) binds the packet.
Reproduce with fresh disk logs using the retained helpers; compile storage
fixtures before their small tmpfs runtime run.

Next is protected managed-stack file/catalog binding before membership can support
complete stack claims or lifecycle planning. Public root/installed qualification
and physical consumer completeness remain separate prerequisites.
