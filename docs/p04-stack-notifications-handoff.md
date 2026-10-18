# Stream B stack notification handoff

The `engineer/p04-stack-notifications` branch adds transport-neutral *arr event
normalization from main `7df3f75`. The domain planner and supplied-body decoder
preserve the frozen event rules. Central ingestion, durable outbox publication,
configuration/API/UI registration and delivery remain pending. This is a B4/RW-042
and RW-045 leaf, not registered notification parity or B1 completion.

Tested product source: `b72c14cfdf3ca904195ce0403f1f54f5c4cd6b7d`. The
[evidence packet](rewrite-evidence/p04/2026-10-18-stack-notifications/README.md)
records 30 new native tests within 454 passing combined B cases, strict all-target
Clippy, formatting, exact inputs and retained failed attempts. Documentation-only
follow-ups preserve those product bytes.

The [design](plans/2026-10-18-stack-notifications-design.md) records the ownership
boundary. No shared root export, Store/schema, API route, protocol, credential
registry, frontend shell, package or activation file changes here. The two
existing B-owned deployment module lists export the leaves for the combined
harness. Production crate roots still need the integrator's registration.

## Supplied behavior

`crates/domain/src/deployment/notifications.rs` exposes `normalize`, borrowed
`Input`/`Media`, quiet/verbose `Mode`, closed event/severity types and either a
bounded notification or an explicit unknown/test/policy suppression reason.
Notification construction is private, with serialization but no deserialization.
Debug output includes event/severity and text byte counts only. Serialized text
is still untrusted display data; this does not redact secrets embedded in a
sender's selected message or title. Consumers must render text nodes.

`crates/executor-container/src/deployment/notifications.rs::decode` selectively
decodes supplied bytes and calls that planner. It performs no network request,
authentication, source discovery, file access, publication or send. Unknown
fields are skipped. Neither the sender-controlled instance name nor decoded
content proves a provider/container identity or authorizes an operation.

| Frozen behavior | Preserved result |
| --- | --- |
| Download / Import; Upgrade; imported `isUpgrade: true` | Imported/upgraded event and title; upgrade flag affects imported events only |
| HealthIssue / Health; HealthRestored | Warning, or critical for health `level: error`; recovery is informational |
| DownloadFailure / GrabFailure / ImportFailure; ManualInteractionRequired | Failed/manual warning |
| Grab, Rename, MovieAdded / SeriesAdd, ApplicationUpdate | Suppressed in quiet mode; forwarded as data in verbose mode |
| Test; unknown event | Always suppressed, with distinct reasons |
| Movie/year; series/first episode; missing episode number | Movie takes precedence; `S02E03` labels; missing episode number becomes zero |
| File quality selectors then release fallback | Movie, episode, track, book order; retain the frozen distinction between file and release nested-name fallback |
| Instance name / configured fallback | Trim and lowercase supplied instance; preserve configured fallback case/spacing |
| Detail and empty text | First 500 Unicode scalar values, preserving multiline data; `(no details)` fallback |

The [fixture](../tests/fixtures/deployment/stack-notifications.json) transcribes
all 16 event aliases and eight representative payload expectations from frozen
source/tests. It contains synthetic data, not a deployed configuration snapshot.
The test loops cover both policies; they count as native test cases, not one
test per fixture row.

## Explicit input replacements

The frozen implementation coerces some malformed values and ignores non-object
payloads. The new decoder returns bounded typed errors for invalid bodies or
selected field types. Arrays cannot substitute for wire objects. Duplicate
selected keys, including null duplicates, fail. Missing/null optional fields
remain supported. Unknown fields are skipped without retaining their contents;
their container depth still counts toward the input limit.

Limits are 256 KiB per body, 4 KiB per selected string, 128 episodes while
retaining only the first, and 32 container levels including unknown fields. Source
display text is capped at 128 Unicode scalar values; detail at 500. Nonnegative
u16 episode labels accept integers or at most five decimal digits; years are
0–9999, with zero omitted. Floats, booleans, negative/oversized labels, invalid
UTF-8, trailing JSON documents and controls other than CR/LF/tab fail explicitly.
These bounds require an integration error view rather than silent truncation of
an entire invalid request. No observed installation has been qualified against
the new limits yet.

## Frozen channel/configuration inventory

This is a source inventory at pi-health
`80593b29443ea9b81eb60ed4e5c3c3ac7a236204`; no reference module/helper/test was
executed. Deployed use and any transport retirement remain P00/P06 decisions.

| Flow/source | Preserved settings or behavior | Remaining integration/migration requirement |
| --- | --- | --- |
| `alert_daemon.py`, `alert_notifier.py` | Alert webhook env reference; missing webhook selects recording/log mode; incident/recovery payloads | Current ordinary notifier/outbox remains usable without a model; retain explicit unavailable/failed/ambiguous external state and incident/recovery ordering |
| `mattermost_integration_service.py::_write_stack_notifications_config` | Version, enabled, token, webhook URL, mode (quiet default), source default (`stack`), channel name; preserves existing ingest token on re-provision | Centrally protected credential references and reviewed configuration migration; no token or webhook URL in ordinary reads/logs |
| `stack_notifications_service.py::status/set_mode/ingest`, `integrations_manager.py` | Authenticated status and mode update; token-gated inbound POST; 256 KiB cap; valid test/noise returns ignored; delivery failure is separate from forwarded | Central token authentication/revocation and admission limits; POST/CSRF/current grants/revision for user configuration; configured is not verified delivered |
| `stack-notifications-card.tsx`, `frontend/src/lib/integrations.ts` | Status, quiet/verbose toggle, ingest URL copy, asynchronous existing-Mattermost setup | Native UI acceptance still required; provisioning/chat transport remains optional, never a prerequisite for in-app notifications |
| `mattermost_integration_service.py::_write_package_updates_config`, `limeos_packages.py` | Separate enabled/package-updates webhook/channel; held-update payload preparation | Registered package reconciliation events need central durable notification receipts and migration; this leaf does not cover that renderer/channel |
| `update_service.py` | `notify_on_update` (true default) retained in container update settings | Bind accepted deployment outcomes to the notifier; a saved flag alone proves no send |

No channel is retired or marked migrated. This inventory does not claim to cover
every legacy notification producer or schedule. Mattermost installation and chat
transport stay outside this leaf; preserving event semantics does not accept a
new mandatory transport dependency.

## Central handoff and acceptance

Consume this decoder only behind the accepted central ingestion contract. Resolve
the current configured sender, protected token and revocation policy before
admitting an event; caller text cannot select a principal or stack. Bound request
and queue admission before buffering. Keep configuration mutation authenticated,
revision checked and CSRF protected. Reads must not send, normalize saved files
or advance delivery state.

Assign the durable event/admission identity centrally and reserve it atomically
with outbox admission. Normalized source/title/detail are not deduplication keys.
If the sender supplies no reliable event identity, document the retry/duplicate
semantics; identical bodies can represent separate legitimate events. This leaf
creates no occurrence, receipt or alternative registry. Existing B1 delivery
helpers govern retries and ambiguity only after the accepted Store transaction
and current authority checks are present.

The qualification owner's installed scenarios must include all alias/policy
fixtures through API/UI with models disabled; bad/revoked tokens; wrong grants,
origin and CSRF on configuration; quiet/test suppression; source spoofing;
invalid/oversized/deep bodies; full outbox rollback; concurrent admission and
restart; stale policy changes; failed publication; and external ambiguous sends.
Enabled configuration with a missing destination must remain blocked, rather
than inherit the frozen `forwarded` result without a poster call. A successful
ingest response is distinct from verified notification delivery. Existing
Mattermost setup/status/mode/copy flows and package update notifications retain
their own acceptance obligations if preserved after inventory decisions.

Focused and combined local check results are recorded in the accompanying
source-bound evidence packet. No installed, ARM64, package, browser, performance,
crash-recovery, defect closure or P04/cutover acceptance is claimed here. Existing
schedule-registration and storage worktrees remain untouched.
