# Prove a refused mount guardian never launched its effect

Baseline `c000939` registers B1 observation supervision. A1 still refuses normal
process/FD churn conservatively. The latest installed evidence at
`2026-10-30-a1-core-fd-refusal` records a bound guardian, Root outcome unknown
with pending quiescence and matching retained Root/Core claims. The release-time
refusal stage is inferred from that evidence; no historical GO or termination
proof exists. These existing uncertain actions remain barriers.

Current source explains a recoverable future case: after READY and durable
guardian binding, policy/cancellation or admission revalidation can refuse before
the first GO byte. Dropping the gated owner closes stdin, but the guardian exits
without an authenticated completion frame. Its cleanup monitor cannot establish
quiescence, so the process-local flight and durable claims remain uncertain.

## Approach

Extend the existing guardian outcome with `not_started`. Only the fixed guardian
can emit this ACK while it is still before its mount-capable child spawn. The
parent accepts it only from its original owned child, with exact action/digest,
complete bounded framing, successful guardian exit and actual wait/reaping.
This supplies a past termination fact, never launch permission. Reuse the
existing `Reaped { helper, outcome }` structure and helper-identity binding;
`not_started` is quiescent and never successful.

Closing a pipe alone cannot prove that GO was not delivered, and a dead guardian
alone cannot prove its descendant is dead. Neither observation supplies this
outcome. Relaxing global consumer comparisons or exempting Core/Docker would
change safety semantics and is outside this correction. Keep all current
identity, membership, retained-owner, lifetime and resource checks.

## Guardian and parent ownership

Extract the actual bounded GO parser so the production role and native protocol
fixtures exercise the same implementation. After READY, any refused/truncated/
expired GO, closed input, parent disappearance or failed pre-spawn check can
acknowledge `not_started`, because no child exists. A successful full GO still
requires both original deadlines, exact binding and the existing final host-view
and parent checks before spawn. After a child is spawned, only its existing real
exit/signal plus descendant cleanup can produce a normal ACK. No post-spawn path
may turn failure into `not_started`.

Give the parent an explicit consuming gated abort. It closes input and waits for
the original child within the existing three-second supervision window plus
250-ms refusal margin. It accepts only a genuine `not_started` ACK. Missing,
malformed, mismatched or unsuccessful completion retains uncertainty and the
existing conservative cleanup ownership. Cancellation/drop still cannot mint
termination proof.

Separate owned process I/O from retained admission/image ownership internally so
native unprivileged child fixtures can exercise the real framed completion path.
Production construction still requires the fixed sealed image, root host-view
identity and genuine admission. No production endpoint, executable, credentials,
clock or test selector becomes caller-controlled.

During release, collect pre-write refusal separately from GO publication. A
pre-write refusal attempts the explicit abort and returns its genuine quiescence
to orchestration. Once the first GO write is attempted, any write/flush failure
remains potentially delivered and cannot use the parent's gated-abort proof.
The ordinary running-helper path may observe the guardian's actual outcome;
otherwise it retains pending uncertainty. A guardian may truthfully report
`not_started` after a full GO if a final pre-spawn check refuses.

Recheck the actual Root write ceiling after awaited admission/identity
revalidation, immediately before GO publication. Then recheck cancellation and
both original deadlines, with no further await before the first write. This
trusted local authorization callback is not wire-supplied proof. A revocation
during the earlier wait must use the same authenticated abort path. Native
owned-pipe fixtures exercise this final authorization/deadline publication
boundary, separately from installed protected-policy acceptance.

## Durable recovery

Orchestration persists the genuine reaped outcome through the existing journal
transition before attempting fresh unmounted reconciliation. Root publishes
`precondition_changed` and releases its claims only if the original plan's exact
unmounted target/device/configuration/consumer evidence is freshly re-established.
If that read refuses or cancellation expires, preserve `outcome_unknown` with
the saved reaped proof and all claims.

Extend the existing read-only reconciliation request to a historical
prepared/unknown receipt with genuine saved quiescence, no accepted mounted
result and a non-success outcome. Current protected read policy and exact target
identity must still bind. A later fresh unmounted observation can settle that
same action without launching, renewing expiry or creating a replacement job.
Pending/unknown quiescence remains forbidden. Existing accepted-effect
verification retains its behavior.

Core may request this narrow reconciliation for a correctly bound quiescent
non-success receipt. It still requires its own separate fresh unmounted proof
before recording terminal no-effect and releasing Core claims. Interrupted
publication, restart and same-action replay never execute another mount.
Old pending receipts gain no proof from this change.

## Qualification and scope

Use real owned child pipes and pidfds to test READY/GO/closed/partial/expired and
mismatched frames, successful wait, malformed or absent ACK, post-GO ambiguity
and no mount-capable fixture child on pre-launch refusal. A native fixture's
effect is an owned marker, never a mount. Assert actual child reaping and keep
its scope distinct from installed root/image acceptance. Add domain, journal,
Store and Core recovery regressions for exact binding, non-success quiescence,
current policy, original expiry, interrupted publication and unchanged pending
barriers. Regenerate contracts and run appropriate workspace/opt-in, strict
Clippy, formatting and repository checks.

Use repository-disk scratch, one Cargo job/test thread, four-GiB compile starts
and the unchanged sampled three-GiB stop floor. Preserve source-bound attempts
and process identities. Fresh package/installed disposable-VM qualification is
still required for the new outcome. Normal host churn/completeness reliability,
successful mount/UI/replay/restart, full resource behavior, ARM64/Pi, the other
A/B/P04 milestones and P05/P06 remain open. No Pi, real host mount, service
activation, existing uncertain-action repair or push is authorized by this slice.
