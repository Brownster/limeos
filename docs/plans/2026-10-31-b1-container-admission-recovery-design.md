# Admit container observation work before protected collection

Baseline `8c04e24` has a genuine protected health transport and partial Store
completion, but Core reads before checking due/current authority. Stable
not-due polls unnecessarily contact the reader. Existing queued observation
jobs cannot yet consume the fresh cohort with its original deadline and current
concrete bindings. Address these runtime ordering/recovery requirements before
normal fair scheduling and remaining health-source registration.

## Admission first

Add one private-field, nonserialized container-read permit wrapping either the
existing volatile poll lease or original queued observation lease. It exposes
only its original deadline. Constructors remain inside the Store. Use the same
current schedule, principal/grants, policy revision, due/watermark, oldest-job,
generation, owner single-flight, capacity and deadline admission rules that
already govern those leases. Initial admission checks host health-read and
observe-health management authority before Core contacts the root reader.

Preserve the existing poll dispositions (not due, paused, rollback, skipped).
A skipped occurrence keeps the existing atomic settlement. Existing durable job
results are returned without collection. A pending permit conveys no authority
for unknown container scopes; the fresh cohort supplies their actual full IDs.
In the completion transaction, bind every actual member plus host scope and
reauthenticate all of them, alongside the original principal/policy/schedule
revision. Missing grants refuse the whole sample.

An advisory query followed by later admission would repeat work under races
and lack the existing owner exclusion. Guessing container scopes before reading
would trust stale inventory. The selected permit reuses existing exclusion while
deferring only concrete source binding, whose checks stay inside completion.
The old generic leases/APIs remain available with their existing semantics.

## Fresh completion and queued recovery

Core starts the existing at-most-four-second read window only after admission,
bounded by the permit's original deadline and caller budget. The protected
nonce/socket/peer/report checks remain unchanged. Carry the original read
deadline through the actual database queue and transaction. Neither a report
copy nor scope binding renews the original admission or Engine lifetime.

Reuse one transactional container-binding check for polling and queued jobs:
exact current member scopes, original wall age, full-ID keys and explicit
refusal of legacy silences or absent retained subjects. Both paths create only
globally incomplete container samples. Queued completion consumes the original
job/occurrence and sequence, folds alerts/outbox and finishes atomically; it
creates no replacement occurrence. Duplicate durable results remain idempotent.

On read/completion failure Core attempts bounded release of only its own permit.
Cleanup cannot settle a job, commit a sample, renew authority or release newer
work. A cancelled/expired caller can leave a volatile lease until its original
expiry; subsequent admission already removes expired leases. Preserve this
honest limitation rather than starting detached unbounded cleanup.

For an expired or authority-invalid queued job, use the existing discard path
before collection. It independently rechecks the original job, oldest ordering,
clock and current authority and records the existing failed/precondition-changed
result only when justified. A source failure leaves a still-live original job
queued for fresh retry. A restored authority/clock race cannot authorize an
unconditional discard. Expose a trusted internal Core recovery method for one
job; fair page/cursor supervision and timer activation remain a subsequent step.

## Qualification and boundary

Exercise actual Store admission and Core RPC fixtures: zero reader requests for
not-due/paused/unauthorized/expired work, unchanged zero-write polls, original
owner exclusion, missing concrete grants after collection, policy/revision
changes, failed-reader release and cancellation expiry. Prove queued original
identity survives reopen, completes once, publishes through the existing in-app
path, and invalid old work is discarded without contacting the reader. Include
actual database queue/write-lock waits after collection and original deadline
checks; retain existing stale wall/source/legacy/retirement regressions.

Run affected/full workspace tests, strict relevant all-target Clippy, actual
contract drift, formatting and repository hygiene against the same final input
map. Public wire schemas, SQL schema 14, package policies, service ceilings and
frontend behavior remain unchanged. Use repository-disk scratch, one Cargo job
and test thread, 4-GiB starts and the unchanged sampled 3-GiB floor; preserve
failed attempts and original native process identities. No Pi, Docker, installed
service, external recipient or push. This advances the real B1 runtime path;
remaining health sources, fair scheduling, installed qualification and full
A/B/P04/P05/P06 cutover remain required.
