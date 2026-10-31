# B1 configured health policy

Source `a5c8378` implements the
[configured-policy design](plans/2026-10-30-b1-configured-policy-design.md).
[Local evidence](rewrite-evidence/p04/2026-10-30-b1-configured-policy/README.md)
qualifies registered settings and their existing Store integration. Genuine
protected collection and production supervision remain unregistered.

## Registered configuration

GET `/api/v1/automation/health-policy` reads the current owner's configuration.
POST to the same path replaces its complete definition by expected revision.
Both require current host-read and the narrow health-automation management
grant. POST also requires Origin and CSRF. The sole Database worker rechecks
authority and the original ten-second deadline inside its transaction. A late
or abandoned reply can follow an actual commit; the UI reconciles by owned GET
without automatically repeating the POST.

An implicit default is revision zero and creates no row or event on read. The
first explicit replacement creates revision one even for an empty/default
policy. A same-definition update to the current explicit revision performs no
write. An explicit replay of its immediately preceding replacement returns the
existing committed revision; other stale replacements conflict. Concurrent
different replacements at the same revision commit one winner.

The generated definition preserves threshold, disabled categories, literal
required paths and silence key/creation/expiry/reason metadata. The qualified
[frozen planner](p04-b1-health-settings-migration-handoff.md) can project supported
settings without dropping that metadata. Poll interval remains schedule metadata;
a future actual-state migration adapter must apply the planner's interval through
schedule creation. This endpoint reads no deployed environment/configuration,
credential or external destination and grants no concrete resource authority.

Definitions retain existing runtime limits and the explicit timestamp profile:
threshold 1–16, five categories, 256 distinct required mounts, 64 silence rows,
256-byte keys and 200-character reasons. Paths fit their literal mount alert
key. Creation timestamp text and fractional precision survive; nonzero fractional
expiry refuses rather than being rounded. Definitions are capped at 64 KiB;
HTTP envelopes allow a further 4 KiB. Invalid UTF-8 is refused by the new client.

## Durable binding and publication

Schema 13 appends immutable owner/revision policy records with canonical body,
digest, trusted update time and an authority event. Startup checks the compiled
schema/triggers and validates bounded bodies, contiguous owner revisions,
ownership, times, digests and event proofs before recovery. At most 1,024 policy
versions are retained globally; a full history refuses a new version without a
partial row or audit. Safe policy-history compaction is not implemented here.

Occurrences, observation results and streams now carry the bound revision.
Reservation binds it atomically with the original occurrence/job. Startup checks
the policy current before the durable JobQueued event, including equal wall
times. Old schema-12 state remains revision zero without invented policy events.
Skipped/unavailable effects retain zero and gain no execution adapter.

A private observation lease exposes its immutable bound definition. Changed
policy invalidates unfinished collection; verified precondition-changed discard
retains its original occurrence/job and creates no replacement. Completion
rechecks current policy, grants, resources, generation, sequence and original
lifetime, then folds under the bound definition. Muted incidents remain stored;
notification eligibility resumes at exclusive expiry or re-enablement on a
later genuine observation. Historical results retain their original definition.
History remains partial until a current-policy observation settles.

In-app publication checks current grants and the latest category/silence
predicate in the same transaction as publication. Blocked pending rows remain
visible, with incident-before-recovery ordering intact. Configured publication
adds a policy-check event before its published event; startup verifies the
revision through durable event order. This introduces no external sender or new
exactly-once delivery claim.

## UI and local checks

The authenticated dashboard opens a separate alert-policy editor. It preserves
untouched silence metadata, literal path case and saved revisions. New silences
and removals remain draft changes until explicit save. Refresh/focus/online
perform GET only. Principal, grant revision or CSRF changes dispose the prior
owner, abort transports and fence late replies. Unavailable data disables save;
forbidden/expired access clears retained configuration. Saved settings remain
distinct from unavailable automatic observations.

The final affected run passes 791 runtime tests and nine compile-fail doctests:
25 API, 44 Core unit, seven Core integration, 562 domain and 153 persistence
runtime tests. Eighteen new native tests cover configuration, CAS/races,
deadline/reply handling, policy changes, publication filtering and proof checks.
Compiled schema-11/12 upgrades and the older migration/recovery suites pass.
All 242 frontend tests, the checked production build, strict Clippy, actual
contract regeneration, formatting, repository hygiene and scoped Ruff pass.
The final closed browser fixture verifies three explicit saves: a verified
reply, a committed HTTP expiry and a committed lost transport reply, with no
automatic repeat POST. Desktop/mobile screenshots were visually inspected.
All five final guards bind the same 718 committed inputs; 133 frontend/contract
input hashes and three served UI asset hashes agree.

Evidence retains failed drafts, original guard outputs, source/asset bindings and
desktop/mobile screenshots. Checks use repository-disk scratch, one Cargo job,
one test thread and the unchanged 3-GiB sampled floor. Holly's Pi was untouched;
no package/service activation, external send or push occurred. Existing schema-12
packages and historical ARM64/Pi measurements do not qualify this source.

## Remaining cutover work

Resolve required paths and legacy keys against concrete protected resource
identities, including replacement and retirement of an old binding. A configured
path is not evidence of a current mount. Connect genuine fresh restart-policy,
healthcheck, SMART, required-mount and protection facts; overview cache values
cannot substitute. Supervise bounded reservation, queued collection, publication
and retention with fair cursors and restart recovery.

Protected actual-state migration, destination references/revisions, ordinary
external notification parity, policy-history compaction and installed API/UI
concurrent-tick/restart/24-hour idle-write evidence remain open. Schema 13 requires
fresh package/upgrade and return-path qualification, including a preserved
pre-upgrade database for a return to older code. A1 reliability, broader A/B/P04
parity, P05's minimal assistant and P06's actual-state migration/return rehearsal remain required
before replacing Holly's frozen app.
