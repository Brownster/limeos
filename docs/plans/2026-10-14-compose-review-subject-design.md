# Compose previews bound to live review subjects

Base: local main `613b990`. Existing P03 Compose approvals are durable preview
approvals, with no executable Compose request. Connect those ordinary stored-preview
facts to the actual retained lifecycle proposal to provide a distinct, concrete
subject for later fresh approval. Keep this an unregistered read-only library.
Core/Store, contracts, dependencies, grants, acquired locks, recovery and activation
remain unchanged. No existing preview approval gains executable authority.

Consume an actual `BoundComposeLifecycleProposal` and borrow `PlannedCompose` input.
Check original readiness/window first, then fixed current wall time and P03 scalar/
expiry validation. Require selected stack, sole recognized operator name/raw hash,
fixed managed name, canonical before/desired projects and complete impact to match
the genuine private proposal exactly. Compare bounded genuine models before input
serialization or cloning. Refuse mismatched/unbounded collections and changed numeric
ownership, resources, privileges or shutdown intent. Stream the canonical plan into
a bounded SHA256 writer (48 KiB, matching existing Store) and verify the original
P03 digest and canonical desired/template digest before cloning the preview.

Derive required P03 scopes: deployment_manage on the actual selected stack, plus
deployment_elevated on the exact stack/template/canonical desired digest when the
desired project is elevated. Requirements are facts, not granted permission.
Caller-supplied previews remain ordinary input: this library cannot authenticate a
Store record, principal, current revision, grant, task or human session. The future
composition root must resolve those through the existing authenticated Store path.
There is no approval-token input or receipt API.

Compute a separate domain-separated review digest over the exact preview, required
scopes and borrowed stable lifecycle facts. Include original registration, protected
catalog/path/root/namespace/ancestors/file identities and raw hashes, ordered selected
file observations and recognized absence, complete Engine identity/membership,
selected indices, admitted sources, exact artifact hash/length, full compiler impact,
source changes and every unresolved gate. Exclude only collection observation times;
keep file modification/change times, P03 creation/expiry and incarnation timestamps.
Thus a fresh observation of identical facts need not change the review subject, while
host/content/selection/actor/revision/template/intent changes do. This digest is a
review identity, not a lease, permission, proof of physical completeness or approval.

Retain the original private proposal, bounded preview copy, scopes and digest. Expose
one borrowed serialize-only summary containing all original lifecycle observations,
preview, scopes and review digest; count the whole compact report against 256 KiB.
Keep all original child/artifact limits and at most 38 retained FDs, without a new
FD or lease. Hash stable material through a bounded writer without allocating a
serialized report. Owner and summary cannot deserialize; no child-owner getter.

Construction revalidates genuine original evidence before return. Revalidation and
current-preview/digest comparison retain the original four-second window and child
ledgers, check fixed current P03 expiry, genuine child evidence, stable digest and
whole-report bounds before success. Current-preview input must match the stored
private preview exactly, including principal/revision/id/catalog/template/expiry.
It is comparison only, without Store authentication or new authority. Use a new
atomic guard: failure, cancellation, drop, timeout, concurrency, reentrancy or unwind
permanently invalidates the subject; a successful child cannot restore it.

Local protected scratch-root and authenticated owned Unix-listener fixtures cover
canonical preview/subject binding, P03 and review digest separation, exact elevated
scope requirements, before/desired/impact/operator/stack/template/principal/revision/
digest/expiry mismatches, immutable caller copies, stable observation-time exclusion,
changed meaningful material, source/Engine changes, original expiry/read exhaustion,
cancellation/drop/concurrency/reentrancy/unwind, child-isolation and exact report/hash
caps, with owner/report import refusals. Retain complete diagnostic and exact-source
Rust/Clippy/fmt/contracts/repository/local ARM64/pinned parser evidence and hashes.

Build/cache/general scratch stays on disk; existing storage fixtures compile there
before small owned auto-cleaned tmpfs runtime. No Pi/SSH, production Docker, raw block
probe, guest, reserved backup or external qualification paths. Deployed configuration
equivalence, independent daemon/physical identity, physical consumers, authenticated
principal/current grants/fresh executable approval, acquired claims/locks, writes/
recovery, shutdown/effect-time checks, worker integration, independent review and
public root/namespace/installed/resource/performance qualification remain open.
The signed ARM64 candidate, P04 and cutover gates remain unchanged.
