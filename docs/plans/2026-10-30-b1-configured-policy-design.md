# Bind configured health policy through the existing runtime

Implemented by `a5c8378`. The
[handoff](../p04-b1-configured-policy-handoff.md) and
[local evidence](../rewrite-evidence/p04/2026-10-30-b1-configured-policy/README.md)
record the registered boundary, exact source/asset joins and remaining gates.

Baseline `141fc51` has registered owned history and a pure frozen-settings
migration planner. Observation settlement still uses `AlertPolicy::default()`.
Replace that default-only boundary with owner-scoped, revisioned policy and
registered settings access. Keep the existing schedule interval as the poll
setting; a migration adapter must apply the planner's interval through schedule
creation, rather than save a second ignored interval.

## Definition and authority

Add generated domain types for a complete health-policy definition, an owned
view and compare-and-set replacement. Preserve threshold, disabled categories,
literal required mounts and silence creation/expiry/reason metadata. Reuse the
qualified timestamp profile; reject unsupported precision, invalid/duplicate
mounts and excess bounds. Stored and HTTP definitions stay below 64 KiB. The
frozen planner can project a supported candidate into this definition without
discarding metadata. It grants no source, credential or resource authority.

GET `/api/v1/automation/health-policy` returns the current owner's configuration.
POST to the same path replaces it with current session, Origin, CSRF, host-read
and the existing narrow `automation:observe-health` management grant. The sole
Database worker applies the original ten-second deadline before queueing,
inside the transaction and after its response. The initial implicit default is
revision zero and performs no write on read. The first explicit replacement,
including an explicitly empty default policy, creates revision one.

Replacement requires the expected revision. A same-definition update to the
current explicit revision does not write; an explicit replay of the immediately
preceding replacement returns its already-committed revision. Other stale input
refuses. A lost response triggers an owned GET, never automatic POST repetition.
Policy controls collection configuration and delivery filtering only; it cannot
expand read grants, authorize a scheduled effect or select credentials.

## Persistence and observation binding

Schema 13 adds immutable owner/revision policy records with canonical body,
digest, trusted update time and a matching authority event. Retain at most 1,024
versions globally; excess refuses explicitly. Startup validates bounded bodies,
contiguous revisions, digests, ownership, times and event proofs. Old schema-12
state remains revision zero without fabricated historical policy events.

Add a policy-revision column to occurrences, observation results and streams.
Observation reservation binds the current revision atomically with occurrence
and job creation. Skipped/unsupported effects retain zero. A queued job whose
policy changed refuses begin/completion and follows existing verified
precondition-changed discard, retaining its original occurrence and no duplicate
job. The private lease exposes its bound definition to the future genuine
collector. Completion folds only with that definition after current revision,
grants/resources, sequence, original lifetime and generation pass.

Startup joins occurrence policy to the latest policy event before its original
JobQueued event, and results/streams to that bound revision. Historical results
keep their original definition; policy edits do not rewrite old incidents or
receipts. History becomes partial until a current-policy observation settles.
Required-mount strings remain configuration, pending concrete protected binding.

## Publication and UI

In-app publication already checks and publishes within one Store transaction.
Check the latest policy's category/silence predicate at the transaction's trusted
time, preserving incident-before-recovery ordering and existing authority checks.
A disabled/silenced pending item stays visible and blocked; no incident is
erased. When a configured revision permits publication, record that revision in
an additional policy-check event. Startup verifies it against the policy current
before the published event using durable event order, including equal wall times.
No external sender or new exactly-once claim is introduced.

Register a separate authenticated policy editor using generated types, bounded
runtime decoders and owner disposal. Preserve literal mounts and silence metadata
through edits. Expose threshold/category/mount settings and explicit silence
addition/removal. Distinguish saved settings from unavailable new observations.
Current grants remain server decisions. Refresh/focus/online perform GET only;
stale/lost-response/forbidden states disable mutation until reconciled.

## Qualification and remaining work

Use real file-backed Store and Core/router tests for CAS/replay, owner isolation,
current grants/CSRF, query-only reads, expired queued work, post-commit lost
replies, configured threshold/silences, revision changes during collection,
publication-time changes, restart and schema-12 upgrade. Tampered revision/body/
event/result/stream bindings must refuse startup. Qualify generated contracts,
frontend behavior and desktop/mobile rendering under repository-disk scratch,
one Cargo job/test thread and the unchanged 3-GiB sampled floor.

This registers configuration and binds it to existing Store behavior. Genuine
protected collection, resource replacement, bounded production supervision,
external destination revisions/senders, installed concurrent-tick/restart/24-hour
idle writes, A1 reliability, broader A/B parity and P05/P06 cutover remain open.
No production Pi window, deployment or push follows from this design.
