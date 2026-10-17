# B1 Store foundation

This bounded increment persists automation schedules and occurrence reservations
through the existing Store. Observation dispatch remains refused. It registers
no timer, API route, UI, collector, notification publisher or service.

At initial assembly on 2026-10-17, this foundation was based on
`3311f762bbec95d45e1056ad554cf51bfa0281b3`, retaining the published A1 qualification
changes. Nine unchanged B1 dependency files were copied from `7df3f753`; unrelated
Stream B history is excluded. The corrected 21-file foundation packet and those
nine dependencies were assembled with this note: 31 changed paths. All 14 existing
tracked preimages matched both bases before copying. Cargo manifests, lockfile
and external dependencies remain unchanged.

| Verification item | Status |
| --- | --- |
| Owner's final 21-file manifest | `998fd7da991d4a61849a9e576c9076f61f02506d00f363fbb8cda5e81f7db025` |
| Assembled source equivalence, hashes and modes | Verified at initial assembly |
| Rust tests or compilation in this isolated candidate | **UNRUN** at initial assembly |
| Hosted CI and installed acceptance for this foundation | **UNRUN** at initial assembly |

The checks below ran in the owner's `7df3f753` worktree before assembly. They were
not new executions in this isolated candidate. Source comparison confirmed the
required inputs; unrelated Stream B files and results were excluded.

| Recorded check | Actual scope |
| --- | --- |
| Earlier focused libraries | 171 domain and 4 policy tests passed on unchanged final inputs; the earlier 108 Store result predates the audit corrections |
| Corrected Store | 112 tests passed with final business logic; the later four-line fixture-accounting correction passed its exact affected test separately: 1 passed, 111 filtered |
| Strict Clippy | Final persistence and schema producer, all targets; other Rust crates retain the earlier workspace check |
| Generated contracts | Actual Rust producer emitted 58 outputs, six changed; independent regeneration, TypeScript checking, repository hygiene and whitespace passed on final outputs |
| Rust formatting | Original changed-module roots passed; one read-only final-test check covered the later four-line accounting correction |

The optional pre-fix audit regression did not execute: its attempts failed to
compile or were refused before execution. No mutation-test proof or new combined
workspace total is claimed. No B1, P04, defect, resource, production or cutover
gate closes here.

The schema 11 migration adds two tables:

- `automation_schedules` retains owner and grant revision, fixed `health-v1`
  policy, operation, server-anchored definition, schedule revision, pause state
  and a settled watermark.
- `automation_occurrences` uniquely identifies `(schedule, revision, due)` and
  records an observation job, a skipped result or an unavailable adapter.

The foundation reuses the sole DB writer, IMMEDIATE transactions, existing jobs,
resource registry and audit helper. It adds no observation-stream, attempt,
alert, outbox or notification tables. Admission caps are 128 schedules, 4,096
detailed occurrences and the existing global 64-active-job limit. A full bound,
audit failure or DB error rolls back the whole reservation.

Create, pause/resume and reserve reload current stored identity, role, grant
revision and grants inside the transaction. Management requires both
`HealthRead(host:system)` under existing read-policy semantics and the literal
`AutomationManage(automation:observe-health)` grant. Administrator and Operator
roles supply the management ceiling; a displayed role alone grants nothing.
Automation management rejects wildcard grants. Enrollment, legacy import and
default grant lists remain unchanged, so existing accounts receive no implicit
automation management authority.

Creation replaces the submitted anchor with trusted server time. Management
uses an expected schedule revision; a revision change cancels old queued
observation jobs atomically. Resume resets the anchor and watermark and binds
current owner authority. Reads return owner-scoped metadata after current
`HealthRead(host:system)` authorization, with at most 32 rows and a 64-KiB page
bound. Reads do not advance watermarks, prune history or revise grants. An
unpaused schedule with an obsolete owner grant revision returns an expiry error
rather than appearing active.

Reservation validates the stored definition and current authority, then looks
up the exact occurrence before calculating a new due decision. A retained tuple
or a current-revision due at or below the watermark returns `Existing`; replay
does not create a replacement job after a lost reply, grace expiry or detail
retention. Old revisions are stale. A new observation commits its deterministic
job key, closed intent, occurrence, watermark and audits together. Skipped and
unsupported operations create no job or effect approval.

Retention cannot delete a pending occurrence or the latest current watermark
row. Older terminal detail may be removed only within the guarded retention
rules; this increment exposes no maintenance path. Jobs and their audit history
remain retained. Startup rejects inconsistent reciprocal occurrence/job bindings
before generation or recovery writes. It binds the sole queued event to the
actual owner and supported time range, checks the immutable observation-only
schedule operation, and requires genuine older-revision pause/cancel audit proof
for pruned canceled detail. Corruption fixtures preserve unrelated foreign keys,
compiled guards and audit-byte accounting so those predicates are exercised.

Only atomic reservation can insert
`Intent::AutomationObserveHealth { occurrence }`. Its resource is fixed to
`system`; that internal resource is separate from the `host:system` read grant.
Generic queue and transition methods reject this intent. SQL and startup guards
limit its jobs to queued or operation-canceled states and reject dispatch,
completion or canceled-job resurrection. The existing global resource lock is
reserved for a later accepted dispatch path; queued reservations do not prove
that execution or cleanup occurred.

Registration still requires reviewed solutions for these boundaries:

- Authenticated grant provisioning: the internal `revise_grants` primitive is
  not an authorizing API. No direct exposure, raw SQL or silent default grant
  substitutes for that missing seam.
- Per-resource read authority: `HealthRead(host:system)` alone cannot authorize
  every container or storage signal a future collector might return.
- Genuine observation provenance and supervision: batches, job IDs and imported
  facts cannot prove current evidence, original deadlines or helper cleanup.
- Status contracts: before API/UI registration, represent authority suspension
  honestly; preserve the current expiry refusal rather than showing it active.
- Runtime and installed qualification: no timer, API/UI integration, collector,
  dispatch/completion, notification delivery, acknowledgment or service
  activation is included in this foundation.

The bounded foundation estimate is 12 human hours, with scope review at 18 hours.
The P04 phase estimate and review threshold remain 320 and 480 hours. These are
planning figures; measured human effort is unknown. Installed B1 acceptance and
all existing A1/BKP/cutover and resource/trust gates remain open. The historical
storage-inventory result of 61.401 ms p95 against a 20 ms ceiling remains failed.
