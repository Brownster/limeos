# A1: approved nonpersistent storage mount

The combined candidate registers one complete source flow for mounting an
existing managed ext4 Data filesystem at a prepared empty target. A user selects
a protected catalog entry, reviews its identity, approves a bounded plan and
queues a durable job. The root executor independently resolves and revalidates
that identity before any effect. The new effect service remains dormant until
its separate profile and disposable-guest behavior are qualified.

This increment does not close A1, P04, BKP, installed-effect, recovery, cutover,
production-trust, resource, latency, Pi or footprint gates. The local evidence
packet records actual checks and source snapshots; historical qualifications
retain their original scope.

## User and API flow

The Storage panel presents a root-configuration/ceiling catalog, a review dialog
with disk ID, UUID, target and expiry, explicit approval, durable job history and
queued-job cancellation. A lost queue response reuses the same proposal, approval
and idempotency key. The dialog traps focus and restores it on close; source and
TypeScript checks do not substitute for a browser or installed accessibility run.

| HTTP route under `/api/v1/storage/mounts` | Behavior |
| --- | --- |
| `GET /targets` | Protected managed Data/ext4 catalog; no readiness authority |
| `POST /plans` | Inspect selected target ID and persist a reviewed proposal |
| `GET /plans/{id}` | Read the caller's bound plan |
| `POST /plans/{id}/approval` | Bind approval to exact digest and fresh evidence |
| `POST /plans/{id}/cancel` | Cancel an unconsumed plan |
| `POST /jobs` | Queue with the exact proposal, approval and idempotency key |
| `GET /jobs`, `GET /jobs/{id}` | Read the caller's durable history/status |
| `POST /jobs/{id}/cancel` | Cancel before dispatch |

Current authentication, StorageManage grants, CSRF checks and grant revision
apply throughout. Catalog/plan collection checks authorization before and after
the protected read. Plans expire within the existing five-minute plan TTL;
execution additionally requires a fresh original four-second admission window.
The operation is disruptive, has a thirty-second deadline and reconciles before
retry. Plans are bounded to 24 KiB and receipts to 64 KiB. The mount queue route
uses a 32 KiB HTTP body cap so a valid reviewed plan fits its wrapper; other
mutation parsers retain their previous cap.

## Durable authority and recovery

Authority schema 10 adds immutable mount proposals and operation-owned results.
Approval tokens are stored as digests. Queueing consumes the exact plan and
approval atomically, binds principal/revision/idempotency and registers the full
resource set: storage configuration, UUID, target and every physical ancestor.
Claiming persists Running and all claims before the root effect request.

The independent root ceiling narrows exact target ID, UUID and path. Core must
authenticate the executor's Unix peer as root; the executor authenticates the
configured Core UID. RPC accepts intent, selection and receipt lookup, never a
caller-written receipt or a serialized proof of helper termination.

Store startup validates proposal/job/resource/result bindings and retained
verification before generation/recovery changes. This recovery is metadata-only:
it performs no hardware read and never redispatches a mount. Interrupted or
uncertain work keeps its complete claim set. Successful dispatched terminal
states require the operation's receipt and independent verification in the same
durable transaction; corrupt or missing proof fails closed.

Receipt replay never creates another helper or renews approval. A narrow root
reconciliation request can refine an already EffectAccepted receipt only when
its recorded helper outcome is reaped/exit-zero and a fresh exact mounted
inspection succeeds. It can operate after execution expiry without acquiring
write permission. Prepared, Pending and Unknown receipts cannot use mount
absence to clear a barrier or mint a new launch permit.

## Genuine effect boundary

`MountAdmission` owns protected configuration, authenticated Engine declarations,
running-process evidence and a retained physical `MountTarget`. The target holds
actual block, prepared-underlay, fixed-program and current/PID 1 namespace/root
descriptors. The producer resolves target ID from the protected contract, checks
the original four-second admission window before and after recollection and permanently poisons any
failure. A digest records stable facts; it cannot reconstruct an owner.

Admission refuses selected boot/swap/loop/shared backing, sibling or holder
aliases, conflicting fstab entries, nonempty/unprotected targets, configured or
observed consumers and unsupported trust/profile. A bounded host proc collector
checks all visible PID/task membership, retained namespace tables and every
non-self TID's cwd, root, FD table and maps. It covers per-thread unshared FD
tables and literal-space/deleted mapped paths. Kernel tasks require actual
PF_KTHREAD/status/empty userspace evidence. The only membership exception is the
actual owned live fixed guardian child with matching image, role, namespace and
cgroup. Unknown, inaccessible, changing or excess observations refuse; this is
not universal proof about every kernel or future unmanaged consumer.

The dedicated guardian runs before Tokio initialization in its own process. It
copies actual invocation descriptors and establishes its session/subreaper
before READY. The journal durably records Pending and the actual guardian
identity before minting a fresh non-Serde, non-Clone one-use launch permit.
Release consumes that permit, matches action/digest/helper, revalidates the
original admission and checks expiry. Guardian checks the unchanged original
deadline immediately before launching the fixed internal-only mount command.

The helper uses ext4 with rw,nosuid,nodev through held descriptor paths. It runs
no shell or external mount helper and performs no format, target creation,
ownership change or fstab write. Actual mount child exit plus complete owned
descendant reaping precede the guardian ACK. Only actual owned guardian wait and
that authenticated ACK mint an opaque quiescence token. A timeout, blocked
helper, parent loss or missing ACK retains uncertainty and live ownership;
ordinary Child wait or an absent mount does not prove no future effect.

After real quiescence, root separately inspects the named mounted filesystem,
persists EffectAccepted and performs another fresh verification before Verified.
Core obtains its own fresh verification before recording/releasing the durable
job. Post-mount observation checks actual UUID/device/mount ID/root and
rw,nosuid,nodev; it observes the filesystem root's existing UID/mode without
reapplying the prepared empty-underlay predicate or changing ownership.

## Initial profile and remaining qualification

The initial source profile permits at most four authenticated running containers,
sixteen declared destination mounts, 256 host tasks, eight mount namespaces and
eight physical ancestors. Its reservation is 225 FDs, including shared baseline,
transient headroom and guardian ownership, under the existing 226 worker model
and 256 service ceiling. Service limits remain 64 MiB and sixteen tasks. Wider
host parity, actual peak descriptors/RSS/tasks and capability persistence need
measurement; the frozen wider host exceeds this initial container profile.

The current build also retains an existing Btrfs source-profile limitation.
On this repository host, mountinfo and statx name different devices for the same
mount ID (`/home`: `0:35` versus `0:36`; `/`: `0:35` versus `0:39`). The source
owner refuses that identity mismatch before final source inspection. Directory
link counts stay 1 when a child directory is added and become 0 on detachment;
the separate real-directory regression assumes a link-count change. The second
local full workspace run remains failed with 17 cases unqualified. Production
predicates stay intact; Btrfs source support remains open in the new build.
Future supported-filesystem hosted CI must establish its own actual results.

The standard package contains an opt-in dormant service with exactly
CAP_DAC_READ_SEARCH, CAP_SYS_PTRACE and CAP_SYS_ADMIN. It requires genuine host
namespace/proc access and the existing safe openat2 readers. Existing reader
units remain unchanged; a missing profile refuses rather than substituting test
trust. Disabled examples ship only as documentation. Shadow omits the effect
unit. Package teardown refuses uncertain unit states or surviving descendants
and preserves all journals/claims; it never records a mount-helper cleanup fact.

See [the dedicated profile](../p04-storage-mounts-profile.md) for concrete unit,
socket/configuration, teardown and guest scenarios. Saved local captures retain
their actual source scopes; the qualification packet will follow the exact
composed-source CI results. Actual disposable-disk success, installed confinement, blocked-helper
recovery and real old-artifact upgrade remain unqualified. Historical inventory
latency of 61.401 ms p95 against a 20 ms ceiling remains failed; backup prepared
descriptor headroom and production policy/workspace trust remain open.
