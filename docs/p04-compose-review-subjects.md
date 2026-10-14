# Compose previews bound to retained review subjects

`BoundComposeLifecycleProposal::bind_review(preview, cancelled)` consumes an actual
[retained lifecycle proposal](p04-compose-lifecycle-proposals.md) and associates
ordinary `PlannedCompose` input with its genuine private source/artifact/Engine facts.
The result is unregistered read-only library logic; no CLI/RPC/route, job, service,
grant, approval or package activates it.

The [authenticated Store resolver](p04-compose-authority-reads.md) now supplies the
actual current session/task principal, stored preview and complete exact requirements
as a read-only observation. Root broker/transport composition and durable fresh
approval remain open; an earlier successful read is no permission lease.

The P03 [preview approval](p03-compose-planning.md) remains preview-only. This owner
accepts no approval token and provides no execution or receipt method. An input
preview is an ordinary typed value, without authenticated Store or session provenance.
The future composition root must resolve the current owned preview and principal,
revision, grants, cancellation and catalog through the existing authenticated Store.
Library equality and digest checks cannot substitute for those decisions.

Before cloning input, require fixed current P03 validity/expiry, selected stack, one
recognized operator name/raw hash, fixed managed filename, canonical before/desired
projects and full impact to match the actual proposal exactly. Compare genuine bounded
models before serializing caller input; malformed/oversized differing collections
refuse. Stream the canonical plan through a 48 KiB SHA256 writer, matching the existing
Store bound, and verify its original digest plus canonical desired/template digest.
The P03 catalog digest and actual management catalog hash remain separate facts.

Derive `deployment_manage` on the actual stack and, when desired configuration is
elevated, `deployment_elevated` on the exact stack/template/canonical desired digest.
These are the existing P03 scope requirements, without a grant or policy decision.
Fresh grants/roles and task subsets still go through policy/Store. No wildcard or
container/media permission is introduced as deployment authority.

The separate domain `limeos.compose.review.v1` hashes the exact preview and required
scopes plus borrowed stable lifecycle facts: protected registration/catalog/path/root/
namespace/ancestor/file identities and hashes, ordered file observations and recognized
absence, complete selected/unrelated/non-Compose Engine rows and indices, admitted
sources, exact artifact hash/length, full compiler impact, source changes and unresolved
gates. Include the original Engine socket/endpoint/peer policy, declared mounts and
running PIDs through a crate-private borrowed projection of the same genuine owner.
Declared mounts and PIDs do not establish physical host or deployed configuration
equivalence; the corresponding gates remain open.

Exclude only collection observation times from the digest. File modification/change
times, container incarnation timestamps and P03 creation/expiry remain bound. Genuine
recollection of the same named objects with a new Engine issue time matches; changed
PID or declared mounts require a different digest. This is review identity, without
lease renewal, current permission or approval. Root/namespace/socket/path changes and
meaningful preview/host/intent changes require fresh review facts.

One borrowed serialize-only summary includes the original lifecycle observations,
complete original Engine context, copied bounded preview, scope requirements and
review digest. Count the whole compact report against 256 KiB; stable hash material
uses the same bound without allocating a serialized report. Keep all child/artifact
limits and at most 38 original retained FDs, adding no FD or lease. Owner/report cannot
deserialize, and no child-owner getter is exposed. Bounds do not prove peak allocator/
RSS or installed-worker headroom.

Construction revalidates genuine evidence before return. `revalidate` checks retained
preview expiry and child evidence. `check_current` additionally compares ordinary
current-preview input and the expected review digest exactly; its input requires
authenticated resolution by the future composition root. Original four-second windows
and spent read ledgers never renew. Check P03 expiry before and after genuine child
validation. Failure, cancellation, drop, timeout, concurrency, reentrancy or unwind
permanently invalidates the subject; successful original proposal checks cannot restore
it. Reports and candidate bytes remain past facts after failure. Synchronous kernel
work needs independent supervision, and these checks provide no writer transaction.

See the [design](plans/2026-10-14-compose-review-subject-design.md) and
[local acceptance](rewrite-evidence/p04/2026-10-14-compose-review/README.md).
Successful fixtures use protected private roots and authenticated owned Unix listeners.
Public root/foreign namespace/installed/resource/performance qualification and independent
review stay open. All existing lifecycle gates remain: runtime/physical completeness,
authenticated principal/current grants/fresh executable approval, acquired claims/locks,
atomic writes/recovery, verified shutdown/effect-time checks and worker integration.
Core/Store, contracts, dependencies, guards, activation, the signed ARM64 candidate and
P04/cutover gates remain unchanged. No Pi/SSH, production Docker, raw block probe or
guest was used. Existing pinned parser checks use version/config only, without daemon
effects. Next is authenticated durable review/approval and acquired claim/lock integration.
