# Authenticated Compose review reads: local acceptance

Qualified source `afc1514290f2bf65809eb430842ce910af04d933`, based on local main `e65a686`.
Actual current sessions/tasks resolve owned stored previews and every exact requirement
in a deferred SQLite read snapshot. The existing Core preview-read API uses session
resolution and preserves its original response. See the
[design](../../../plans/2026-10-14-compose-authority-read-design.md) and
[interface guide](../../../p04-compose-authority-reads.md).

**927 Rust cases pass:** 884 unit/integration cases and 43 compile-fail doctests.
The 19 new cases and focused rerun are already included. Full workspace acceptance
enables the existing opt-in local worker fixture feature. **61 local ARM64 harness
tests and six existing pinned parser cases pass**, with strict all-target feature
Clippy, fmt, contracts and repository checks. [checks.json](checks.json) binds exact
committed before/after source hashes, commands, environments and full merged logs.

Sixteen new actual database cases cover session/task parity, ordinary/elevated scope,
query-only purity, unchanged approval/audit/generation/expiry/jobs/resources/locks,
unknown/revoked/expired/preview-approval credentials, task UID/ID/generation/revision/
expiry/revocation and missing elevated scope. Forged or oversized expected data never
replaces the bounded stored row. Exact requirements refuse omissions/extras/duplicates/
reordering/wildcards. Current catalog, cancellation, body/digest/row metadata, grants/
roles/revision, wrong owners and missing records refuse as specified. Old success
remains past data after revocation. Fixed end-time identity/plan expiry, rollback/
clock failure, consistent concurrent WAL revocation, public-clock reads/restart,
exact report bounds/redaction and nested transaction/generation mismatch pass.
One Core API case preserves successful read state and rejects inconsistent expiry
metadata. Two import-refusal doctests prevent owner/report reconstruction.

[diagnosis.json](diagnosis.json) retains all five precommit diagnostics and premerge passing checks. The initial
compile error was an unsized slice in a fixture's existing Sized serializer; the
fixture now passes a reference to the slice reference. The Core regression failed
against the old preview read, then passed with the session resolver. A later fixture
changed the wrong already-elevated template after normalization, and another used
a time before actual session expiry. Selecting the template by ID and reading the
actual cutoff corrected those expectations without relaxing production checks.
The complete 84-case Store suite passed. Another engineer then merged and pushed
private backup reconstruction at `4846a96`; its code is preserved in the combined
qualified source. The earlier 913-case workspace and passing checks remain
premerge diagnostics. Full acceptance was repeated against the combined source. No diagnostic was discarded; these runs
are separate from exact committed-source acceptance and independent review is open.

Public resolution uses fixed wall time without clock overrides. Identity and stored
preview are rechecked at the end, including actual current policy and complete task
subsets. Exact caller preview comparison precedes caller serialization/copy. Stored
revision/expiry columns match the canonical plan. The owned result retains actual
principal/preview/requirements, kind, check time, generation and identity expiry;
no token, session digest, CSRF, password or approval is retained/reported. Count a
borrowed serialize-only observation against 64 KiB, keeping the 48 KiB preview cap.
It is a past observation, not a permission lease or executable approval. Concurrent
changes after its snapshot require fresh resolution. The future broker must resolve
before and after genuine host validation; writes still need human session/origin/CSRF
and new durable operation-bound approval. Tasks cannot approve. Transport peer UID
and trusted protected catalog provenance remain composition-root responsibilities.

Query-only tests and API read-state comparisons prove no logical Store mutation,
including approvals and claims. [compatibility.json](compatibility.json) binds
unchanged schema-9 DDL, contracts, dependency manifests and lockfile. No migration,
new route, executor request, job, approval acquisition/consumption, resource claim,
stack/storage lock, host write, recovery or shutdown is added. Root broker/transport,
fresh durable approval, runtime/physical completeness, acquired locks/recovery,
shutdown/effect-time checks, installed/resource/performance qualification and
independent review remain open. The signed ARM64 candidate stays frozen; no P04/
defect/performance/cutover gate closes. Historical inventory p95 61.401 ms still
exceeds the 20 ms target.

Build/cache/general scratch stayed on disk. Storage fixtures compiled there before
small owned auto-cleaned tmpfs runtime. 50 ms samples observed lower-bound peaks of
86336 bytes/8 files, not exact maxima. Existing pinned AMD64
parser checks use config/version only and a nonexistent daemon endpoint. This
SQLite slice uses local tests and needs no Pi access. The operator permits needed
host tests after 22:00 Europe/London, preserving frozen app/media/mounts/health and
the earlier 07:00 return boundary; no replacement is authorized. No Pi, SSH,
production Docker, raw block probe, guest, reserved backup or external qualification
path was used. Guards remain 256 FDs/64 MiB/zero swap/16 tasks.

[source-manifest.json](source-manifest.json) binds 359 source/fixture/
contract/script files. Fixed-timestamp gzip retains every log byte with raw and
compressed hashes; [artifact-sha256.json](artifact-sha256.json) binds this packet.
Reproduce with retained helpers and fresh work/log/output paths, compiling storage
fixtures before their small cleaned tmpfs runtime. Next is root broker/transport
composition, followed by durable fresh approval and acquired locks/recovery.
