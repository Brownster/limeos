# B1 frozen health-settings migration planning

Source `32265d4` implements the
[bounded migration design](plans/2026-10-30-b1-health-settings-migration-design.md).
This supplies a complete representable policy candidate or a typed review issue.
It prepares configured-policy integration; the running Store still uses its
default policy. [Local evidence](rewrite-evidence/p04/2026-10-30-b1-health-settings/README.md)
qualifies this pure domain boundary separately from registered history.

## Interface and preserved rules

`automation::settings::plan_health_settings` accepts only the selected non-secret
daemon fields, a typed policy-read outcome and an explicit fallback creation
timestamp. It reads no process environment, files, credentials, clock or Store.
The caller must retain original source custody and report a review issue before
attempting a migration. No partial policy is returned on refusal.

Supported daemon values preserve the frozen defaults, invalid-integer fallback,
nonpositive clamping and comma-separated mount selection. Existing runtime
limits remain: poll 30–2,678,400 seconds and threshold 1–16. Legacy one-second
polls and threshold 100 require review; they never become silently changed
defaults. ASCII signed decimal integers with internal underscores are supported;
other numeric spellings outside that profile require review.

Missing/unreadable/malformed/JSON-null policy falls back to environment mounts.
A valid empty object overrides them with an empty selection. Wrapped policy
objects are selected as in the frozen loader. Legacy falsey non-null roots also
mean empty policy; invalid truthy roots and invalid field shapes require review.
Unknown legacy fields retain their frozen lack of effect; a policy-file
`fail_threshold` does not override the daemon field.

Required paths retain literal case, spelling and components; only exact
duplicates are removed. Categories control delivery without deleting incident
facts. Silences retain their literal keys, original timestamp text, parsed
instants and trimmed reasons. Missing creation time uses the explicit caller
value. Duplicate silence rows remain intact. Derived `AlertPolicy` has exactly
the supported threshold/category/silence predicate and passes its real validator.

The explicit timestamp profile supports calendar dates, `T`/space separators,
seconds, up to six fractional digits and `Z` or minute timezone offsets. Created
time retains fractional precision. A nonzero fractional expiry requires review
because the runtime uses whole seconds; it is never rounded. Expiry is exclusive
at the actual UTC instant. Other Python-accepted timestamp profiles and instants
outside the runtime's 1970–9999 range require review.

Policy input is bounded at 64 KiB and 64 JSON levels; each selected environment
field at 4 KiB. The planner allows at most 256 required mounts, 64 silences,
256-byte alert keys and 200-character reasons. Required paths must fit their
future literal mount alert key. It never truncates an oversized selection.
Diagnostics contain only a field, bounded reason and optional row index, without
raw configuration, parser exceptions or credential values.

## Qualification and remaining gates

The affected domain run passes 559 runtime tests and seven compile-fail doctests.
Twelve new tests cover all 21 applicable cases from the existing pinned 32-case
notification fixture, plus structural/limit/integer/path/metadata/timezone/leap
year/fractional-expiry boundaries. Independent Python datetime constants check
UTC conversion; the real `fold_alert` verifies a muted open incident is announced
exactly at expiry. The other eleven frozen cases concern external selection,
provisioning and update preferences and remain outside this planner.

Strict Clippy, actual regenerated contract drift, formatting, repository hygiene
and diff checks pass. The original Clippy boolean-style refusal and the earlier
passing pre-simplification test run remain separately retained. No dependency,
wire contract, SQL schema, runtime registration or frontend source changes.
Checks use repository-disk scratch, one Cargo job, one test thread and the
unchanged 3-GiB floor. Holly's Pi was untouched and nothing was pushed.

A protected migration adapter still needs to read actual deployed settings with
source custody and current authorization. Bind accepted policy/destination
revisions through the Store's observation lease, completion and publication;
preserve silence metadata durably. Resolve concrete resource replacement and
connect genuine protected collection before registering bounded supervision.
Settings API/UI, ordinary external notification parity, installed restart and
idle-write evidence, broader A/B/P04 parity and P05/P06 remain open. A serializable
candidate grants no mutation, collection, credential or delivery authority.
