# Bound Compose lifecycle proposals

`limeos_executor_container::compose_management::sources::lifecycle` connects the
[retained JSON owner](p04-compose-retained-json.md) to the
[managed override compiler](p04-compose-override-compiler.md). This is unregistered
read-only library logic. No CLI/RPC/route, job, service or package activates it.

`inspect_compose_lifecycle_proposal(catalog_path, stack, desired, cancelled)` requires
actual effective UID 0 before callback, desired input processing, target, namespace
or fixed Engine access. Alternatively, `BoundComposeJsonEvidence::propose_lifecycle`
consumes an actual retained source owner and an ordinary desired typed project.
Callers provide no current-source preimage, body, hash, child-owner, root/UID/clock/
endpoint override or cap. Reports and proposals cannot deserialize into owners.

Construction checks original readiness and window before compilation, then uses
the actual admitted operator/current models. The existing compiler bounds desired
input before cloning, normalizes it and re-admits the exact generated managed JSON
to prove the supported combination. Every desired service must belong to the original
selected registration. Registration declares management scope; it grants no principal
permission. The private owner retains the exact candidate bytes and their SHA256.

The borrowed summary contains the original catalog/file/Engine/source evidence,
full compiler proposal/impact/numeric ownership, artifact length/hash and source
changes classified as add/change/remove. Each change indexes every original selected
Engine member for that service, preserving full IDs, replicas, one-offs, stopped and
mountless rows. Unchanged, unrelated and non-Compose rows remain in the complete
original report. These are source changes and observed associations, without proof
that deployed image/mount/port/user/network configuration matches either project.

The compiler's shutdown-required removal names and associated observed members are
candidates, without successful-shutdown receipts, safe absence or deletion authority.
Zero members and no-op source proposals remain observations. No directory, volume
or container deletion, executable command or host effect is introduced. Artifact
bytes returned by `artifact()` remain proposal data, without a write capability.

Count one compact borrowed summary against 256 KiB while retaining all original
child limits and the 64 KiB artifact cap. Individually valid source and plan reports
can exceed the whole limit together and must refuse. At most 16 registered service
changes, 16 total selected member references and 38 original retained FDs; no new
retained FD. Compilation/model allocations are bounded by the existing typed input
rules. These limits do not establish peak allocator/RSS or installed-worker headroom.

Construction revalidates genuine source evidence before returning. Later async
checks use the original deadline and child read ledgers, never a new lease. Source
checks retain protected root/namespace/ancestor/name/identity/hash rules and genuine
Engine reacquisition. Failure, cancellation, drop, timeout, reentrancy, concurrency
or unwind permanently invalidates the proposal. A successful child check cannot
restore it. Borrowed summaries and artifact bytes remain past facts after failure.
No child-owner getter is exposed. Synchronous kernel work needs independent worker
supervision; revalidation is a sequence of observations, without a writer transaction.

Both the compiler's original unresolved gates and explicit lifecycle gates remain
in every report: deployed configuration equivalence, independently identified daemon/
host, complete physical mounts/consumers, principal/fresh approval, acquired stack/
storage locks, atomic writes/recovery, shutdown/effect-time verification and supervised/
installed qualification. Hashes and reports cannot substitute for executable approval.
Core/Store, generated contracts, dependencies, service guards, activation and the
frozen signed ARM64 candidate remain unchanged.

See the [design](plans/2026-10-14-compose-lifecycle-proposal-design.md) and
[local acceptance](rewrite-evidence/p04/2026-10-14-compose-lifecycle/README.md).
Successful fixtures use private protected scratch roots and genuine authenticated
owned Unix Engine listeners. Root-positive, foreign namespace, independently
identified physical host/daemon, installed resource/performance qualification and
independent review remain open. The existing six pinned standalone parser checks
use version/config only, without daemon effects. No Pi/SSH, production Docker, raw
block probe or guest was used. P04 and cutover gates remain open.

The [review-subject composition](p04-compose-review-subjects.md) now associates
ordinary P03 preview facts with this owner, preserving original evidence and deriving
a distinct stable review digest and existing exact scope requirements. Authenticated
durable review/approval and acquired claim/lock integration through recovery, with
fresh checks immediately before effects, remain subsequent work.
