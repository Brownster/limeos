# Admission-first container observation and original-job recovery

Source `2505191` applies the
[admission/recovery design](plans/2026-10-31-b1-container-admission-recovery-design.md)
to the existing [protected health runtime](p04-b1-protected-health-runtime-handoff.md).
The internal Core path now admits work before contacting the root reader.
Its source commit and local acceptance are recorded in the
[qualification packet](rewrite-evidence/p04/2026-10-31-b1-container-admission-recovery/README.md).

The Store constructs a private-field, nonserialized `ContainerReadPermit` from
an original volatile poll lease or queued observation lease. Admission uses
current host-read/manage authority, principal and policy revisions, due/paused/
rollback/skip decisions, oldest queued-job ordering, generation, owner exclusion
and existing bounds. A pending permit grants no authority for unknown container
scopes. Already-settled or paused work, and failed host/manage authority, reach
no protected RPC; existing durable results return without a new read. The original skip path
still commits its bounded occurrence atomically.

The fresh report provides the actual full-ID members. Both completion paths
bind every member and host scope inside current transactional authorization,
with the original schedule/principal/policy revision and nonce. The common
container-binding check revalidates wall age, exact scopes and legacy/retained
subject bindings. Missing grants, legacy names and unsupported retirement
refuse the whole sample. Serialized reports reconstruct no admission permit.

The protected read's at-most-four-second window starts after admission and is
bounded by the original permit/caller deadline. Scope binding changes no nonce,
generation or deadline. Queue and SQLite-lock waits cannot extend completion:
the same narrowed deadline is checked after the lock, after clock sampling and
before commit. Existing generic observation APIs retain their behavior.
Container-only samples remain globally incomplete.

`ObservationStoreBridge::recover_container_job` consumes the original queued
job rather than creating a replacement. Successful completion atomically folds
alerts/outbox, saves the existing result and finishes the same occurrence/job.
An existing result returns unchanged. Expired or authority-invalid admission
uses the old independently checked discard path before collection; a clock or
authority race that no longer justifies discard refuses. Source failure leaves
a still-live original job queued for fresh retry.

Read/completion failure attempts only a 100-ms cleanup of its own volatile
permit. Cleanup neither settles jobs nor renews authority. Cancellation can
leave admission until its original expiry, and subsequent admission removes
expired entries. No detached cleanup or promise of immediate cancellation
release is introduced. Old-generation/old-nonce releases cannot free newer work.

Local tests use actual owned Unix RPC fixtures with private unit credentials
and clocks, real isolated Stores, genuine reopen and actual database queue and
SQLite write-lock waits. They verify zero reader requests before refused/not-due
admission, original identity and one in-app publication, no duplicate receipt,
concrete scope refusal, immediate retry after failed collection and bounded
cancellation expiry. This establishes no successful installed root-positive
Engine or service behavior.

The source-bound packet records 1,712 workspace runtime passes and 54 compile-fail
doctests, strict all-target Clippy, actual contract drift, formatting and repository
checks. Four final guards share 732 committed input hashes. All six successful
attempts remain recorded, including the earlier focused maps before test expansion.
All scratch stays on repository disk with one Cargo job/test thread and the
unchanged sampled 3-GiB floor; original qualification processes are absent.

The SQL schema remains 14. Public wire contracts, generated output, package
policy, service ceilings, HTTP routes and frontend behavior are unchanged.
The earlier package return requirement still applies: preserve policy before-images
alongside the matching pre-schema-14 database and package assets.

Fair page/cursor supervision, timer activation, remaining host/SMART/mount/
protection sources, explicit replacement/retirement, safe history compaction and
ordinary external notification transports remain open. Installed API/UI/restart,
current packages/upgrades/return, ARM64/Pi, resource/performance and real idle
soak acceptance are still required. A1/B1, wider A/B/P04 and P05/P06 cutover stay
open. No Pi, real Docker, external recipient, frozen app or service was touched;
nothing was pushed.

Follow-up `d1ba5f0` registers
[fair owned Core supervision and runtime status](p04-b1-observation-supervisor-handoff.md).
Its separate local packet qualifies the producer and authenticated partial/
unavailable projection. It supersedes the timer/supervision gap above; remaining
sources, installed acceptance and wider cutover gates stay open.
