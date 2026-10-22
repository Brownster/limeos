# B1 observation completion in the sole Store

Implemented in `11843c3` and locally qualified on 22 October; see the
[Store handoff](../p04-b1-observation-settlement-handoff.md) for exact evidence and
remaining runtime/installed gates. The design below describes that Store boundary.

Schema 11 reserves observation jobs but deliberately permits only queued or
review-canceled states. The proposed runner has no concrete completion adapter.
Extend that boundary before registering a producer. Use the accepted post-lock
trusted clock and original monotonic deadline throughout admission and commit.

## Collection and ordering

Admit an internal, non-serializable read lease for the oldest queued observation
of an owner. Reload the job/occurrence, current schedule and owner grants in a
SQLite snapshot. Require the fixed management grant, host read permission and
every concrete resource read scope before collection. The caller supplies
trusted normalized resource identifiers, never a principal or permission bit.
Bound the resource list by the 256-signal limit and reject wildcards/duplicates.
Retain literal case/path resource identities, bounded to 512 UTF-8 bytes;
restricting identifiers to CLI-safe characters would lose existing mount IDs.

The existing sole Store keeps at most 64 short-lived leases in memory, one per
owner. A private random nonce, Store generation, owner grant revision, job,
resource set, expected consumed sequence and original deadline bind each lease.
Read collection occurs outside the DB transaction. Expiry, replacement, pause,
grant change or restart invalidates late collection. Repeated collection after
an interrupted read is safe; it is not an effect retry or a new occurrence.

Allocate the next owner-scoped sequence from its last committed sequence when
admitting a lease. Only completion consumes it durably. Serialization ensures
two schedules of one owner cannot independently number observations or complete
out of collection order. No durable running state or separate attempt ledger is
needed for a read: a crash leaves the original queued job, and a new Store
generation invalidates the abandoned lease. Reject out-of-order queued jobs.

## Atomic settlement

Schema 12 adds owner stream watermarks, compact immutable observation results,
owner/resource-scoped alert material and the in-app outbox. A result identifies
the original job, sequence, outcome, completeness, completion time and canonical
batch digest. Store no raw observation batch or resource list in result history.
Compact result proofs follow the existing retained job/audit lifetime and audit
capacity bound; do not add an unbounded payload ledger.

Each result and latest owner watermark bind the complete current alert material
with a framed SHA-256 digest. Missing or altered clear/streak rows must also
refuse startup. Results count the notifications they created. Audited creation
binds each notification's immutable payload/resource; audited pruning proves
legitimate removal after verified delivery and retention. Reserve audit capacity
for every created/pruned notification as well as settlement. Current alert rows
remain retained under the 256-row bound until a separate audited material-pruning
contract is implemented; do not silently discard closed rows to bypass that cap.

Completion reloads current authority and all selected read scopes, validates
the live private lease and job deadline, and checks its expected owner sequence.
Only trusted scoped signals from that collection can be folded. Scope binding
is an internal collector contract, not JSON input or authority supplied by an
HTTP caller. The protected-read adapter must construct these pairs from genuine
resource identities before any execution is registered.

Consume the sequence, persist material alert changes, reserve unique
owner/key/generation/incident-or-recovery notifications, insert the result,
transition the queued job and audit in one IMMEDIATE transaction. Full alert,
outbox or audit admission and any SQL error roll everything back. Duplicate
completion returns its original result and creates no new event. Unavailable or
absent signals retain the existing incident/streak; partial collection remains
explicit. Unchanged alert material is not rewritten merely to advance a signal
sequence; the owner watermark prevents replay.

Only this operation-owned result permits terminal observation states. Generic
queue/transition APIs and running/verifying/effect dispatch remain refused.
An internal discard path may settle an expired or currently unauthorized job
without observations, alerts or notifications. Its reason must be derived from
stored state, not a caller assertion. Retire oldest queued work first so an
expired job cannot strand an owner's later observations. Pause keeps its
existing audited cancellation semantics.

Startup validates compiled schema/trigger definitions and reciprocal job,
occurrence, result, owner sequence, alert and outbox proofs before generation or
recovery writes. Old schema-11 queued/canceled jobs migrate unchanged. Results
and consumed watermarks are retained, including when old occurrence detail is
pruned. No startup path recollects or republishes an observation as a new job.

## In-app publication and reads

Publish bounded pages through the same writer, rechecking current owner/resource
authority and original outbox grant revision. Use the actual opening receipt for
recovery ordering. Commit the readable in-app row and delivered state together;
SQL failure leaves the original pending row. A fair cursor scans blocked rows
without permanently starving later owners. There is no external-send claim.

Owner-scoped alert/notification GETs use one authenticated snapshot and current
per-resource read grants; reads never publish, acknowledge or renew authority.
Expose incomplete or suspended observation state honestly. Acknowledgment uses
current session/CSRF, a visible delivered row and revision compare-and-swap.
Keep delivered retention bounded and preserve openings needed by open incidents
or unresolved recovery delivery. Pending/failed/ambiguous rows are not silently
pruned. Default alert folding retains the frozen two-failure policy; ordinary
notification settings/channel migration remains a separate parity requirement.

## Verification and registration gate

Exercise actual file-backed SQLite transactions: two schedules/one owner,
concurrent lease refusal, late/replaced/restarted leases, current grant/resource
revocation, real write-lock deadline expiry, partial and unchanged signals,
opening/recovery ordering, queue/outbox/audit/SQL failures and lost replies on
both sides of commit. Corrupt/missing result, watermark, alert/outbox and audit
proofs must refuse startup before generation changes. Test genuine schema-11
upgrade and future-schema refusal. Retain the existing effect-dispatch refusals.

Run focused and full persistence checks, strict Clippy, actual generated-contract
drift and repository checks under the existing 3-GiB host floor. Keep this
implementation unaccepted until those checks pass. Protected collection, Core
supervision, API/UI mounting, installed crash/restart qualification and 24-hour
resource/write measurements follow this Store boundary. No Pi mutation or
cutover readiness is implied by passing local transaction tests.
