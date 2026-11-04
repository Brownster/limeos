# Gated guardian refusal and read-only no-effect recovery

Source `5c10736` implements the
[recovery design](plans/2026-11-01-a1-gated-refusal-recovery-design.md)
for future pre-launch refusals identified while investigating the
[installed A1 barrier](rewrite-evidence/p04/2026-10-30-a1-core-fd-refusal/README.md).
That older run durably bound a guardian and retained pending quiescence with
matching Root/Core claims. Its historical GO/termination state remains unknown;
this change cannot construct missing proof for an existing pending action.

The fixed guardian can now acknowledge `not_started` before its first
mount-capable child spawn attempt. It uses the same bounded original GO parser,
action/digest binding, wall/monotonic deadlines, parent and host-view checks.
After a spawn attempt, failure remains potentially delivered; only the existing
actual child exit/signal and descendant cleanup path can acknowledge completion.
There is no never-started shortcut for a potentially running mount.

The parent owns the original child, pidfd, sealed image and retained admission.
An explicit consuming gated abort closes input, then requires a complete bound
never-started ACK, successful guardian exit and actual reaping. A frame alone,
EOF, cancellation, timeout or disappearance supplies no proof. Missing, malformed,
oversized, mismatched or unsuccessful completion retains conservative cleanup
ownership and uncertainty. The existing three-second supervision window and
250-ms refusal margin are unchanged.

Release separates pre-write validation from the first GO write attempt. A
pre-write policy/cancellation/admission refusal can take the authenticated abort
path. Current protected write policy is rechecked after awaited admission and
identity revalidation, followed by cancellation and both original deadlines.
There is no further await before the first write. From that write attempt, a write/flush error may mean delivered GO
and cannot take that shortcut. An ordinary successful release can also receive
a truthful never-started ACK if the guardian's final pre-spawn checks refuse.

The opaque process token projects `Reaped { helper, outcome: not_started }`
through the existing journal binding. It is quiescent and never successful.
Saving it alone releases no durable claim. Root must separately collect fresh,
exact unmounted target/device/configuration/consumer evidence and recheck current
protected read policy before committing `precondition_changed`. A failed current
read keeps the saved quiescence and uncertainty.

The existing reconciliation request can now revisit a saved prepared/unknown,
quiescent non-success receipt with no accepted mounted result. It retains exact
action/digest/helper binding and original plan expiry; it never launches,
renews authority or creates a replacement action. Pending helpers remain
barriers. Existing accepted-effect verification retains its separate proof path.
Core requests this refinement only for a correctly bound authenticated receipt
and still needs its own fresh unmounted observation before releasing Core claims.
Lost independent verification remains durable and recoverable after reopen.

Native unit fixtures use actual child pipes/pidfds and the production GO/ACK/
wait implementation, with an owned marker as their effect. They exercise closed,
partial, expired, wrong-bound and oversized GO; missing, malformed, oversized,
wrong-action/digest, extra-byte and unsuccessful ACK; a complete ACK while the
child remains alive; and a full GO that may complete but cannot be accepted as a
gated abort. A native owned-pipe case also revokes earlier authorization or
expires freshness at the final publication boundary and observes no fixture
effect. These fixtures do not construct root-image or mount admission trust.
Journal/Store tests separately exercise real SQLite reopen, retained claims,
original replay, wrong helper identity, original expiry and fresh-proof refusal.
Core uses real isolated Store and Unix RPC with private task-local test
credentials, absent in production. Its default root-peer refusal remains tested.

Source-bound local acceptance is recorded in the
[qualification packet](rewrite-evidence/p04/2026-11-01-a1-gated-refusal-recovery/README.md).
It binds 1,736 workspace runtime passes, 55 compile-fail doctests, strict
all-target Clippy, actual contract drift, 244 frontend passes/checked build and
repository checks to 740 committed inputs across six matching final guards.
All eighteen guard attempts and the earlier refused memory admission are
retained. The 81 recorded original guard/command/native-child PIDs are absent.
Scratch stays on repository disk with one Cargo job/test thread and the
unchanged sampled three-GiB stop floor.
It establishes no genuine installed root-image/admission/effect path.

The [4 November installed attempt](rewrite-evidence/p04/2026-11-04-a1-installed-refusal-recovery/README.md)
builds fresh source `e417751` packages `0.4.12+gateabort.1` against the same 740
committed inputs. The frontend's 244 tests/build and all five Debian executables
pass. A canonical 1,024-MiB guest passes 50 prerequisite checkpoints and three
negative mount API cases, then refuses the positive plan on one new task under
the authenticated Docker daemon PID. No positive job or guardian execution is
reached. This qualifies current package installation and the recorded predecessor
checks, while the new outcome remains installed-unproven. All eight recorded
qualification PIDs, the owned build container and VM scratch are absent; no
sampled memory stop occurred. Normal task/FD churn needs a structural design
before another candidate. Genuine mount effect/no-effect, current policy,
whole-service resources, API/UI and replay/restart remain required.

The additive outcome changes the executor receipt schema and generated types.
SQL schema 14 and Root journal schema 1, dependencies, service/resource ceilings
and every consumer comparison remain unchanged. Older closed readers cannot
interpret the new outcome: package return needs matching binary/contracts and
Core/Root journal before-images, even when SQL version numbers match. It must
also retain the existing policy and pre-schema-14 database/package requirements.

Normal complete-consumer reliability under host task/FD churn remains open.
This corrects refusal recovery, without establishing accepted mount parity or
weakening its safety conditions. A1, B1, wider A/B/P04 and P05/P06 remain open,
along with current package/return, installed ARM64/Pi, footprint and soak gates.
The native source qualification ran no VM or package activation. The subsequent
4 November run activated services only inside its disposable AMD64 guest.
No Pi, real host mount, existing uncertain-action repair, external send or push ran.
