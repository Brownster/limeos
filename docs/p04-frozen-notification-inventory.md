# Frozen ordinary notification rules

This inventory binds source `pi-health@80593b29443ea9b81eb60ed4e5c3c3ac7a236204`
to 32 captured configuration cases and 25 source hashes. The
[fixtures](../tests/fixtures/automation/frozen-notification-settings.json) were
produced by [selected source definitions](../tests/reference/frozen_notifications.py)
with in-memory repositories, paths and credential slots. The frozen app, helper
module, real configuration and transports were not imported or started. This is
source behavior, not Holly's deployed configuration or notification acceptance.

[Local check records](rewrite-evidence/p04/2026-10-21-notification-inventory/README.md)
retain the 11 passing focused tests before formatting, final Ruff checks and the
full-suite refusal below the host memory floor. They provide no new Rust runtime
or installed qualification.

The [October 22 Store follow-up](rewrite-evidence/p04/2026-10-22-b1-observation-settlement/README.md)
now records all 84 reference tests passing, including these 11 inventory tests
after their final formatting. It qualifies Store settlement separately from
configured-policy and external transport parity.

The [30 October typed migration planner](p04-b1-health-settings-migration-handoff.md)
now preserves representable health settings and silence metadata, with explicit
review for unsupported values. Its source-bound domain tests cover the 21
daemon/policy/expiry/precedence cases here; the other eleven cases cover external
destination selection, provisioning and saved preferences. It does not read deployed
configuration or bind policy revisions to the running Store.

The [configured-policy integration](p04-b1-configured-policy-handoff.md), source
`a5c8378`, now registers owned settings through API/UI and binds revisions to
observation settlement and current in-app publication. Protected actual-state
migration, concrete resource binding, production supervision and ordinary
external destinations/transports remain separate acceptance work.

The earlier [stack notification handoff](p04-stack-notifications-handoff.md)
retains its decoder/event fixtures. This inventory adds executable configuration
and precedence evidence. An in-app outbox alone does not preserve these external
flows, and no existing channel is retired here.

| Flow | Verified frozen behavior | Rust integration still required |
| --- | --- | --- |
| Health alerts | `alert_daemon.py::config_from_env` defaults to 60 seconds and two failures. `alert_notifier.py` renders incident/recovery payloads and uses a 10-second HTTP timeout. Missing webhook selects `RecordingNotifier`. | Owned threshold/category policy is now registered. Connect genuine protected facts and current external destination references. Recording/log mode must not become proof of external delivery. Keep observation and external delivery outcomes distinct. |
| Alert categories and required mounts | `alert_policy.py` defaults container, SMART, mount and SnapRAID delivery to enabled. Paths retain case; normalization removes duplicate exact paths. A valid policy file, including `{}`, replaces environment mount selection. Missing, malformed or JSON-null policy falls back to environment mounts. | Owned category/mount settings and observation revisions are registered; the planner preserves empty-versus-missing precedence. Migrate actual source state and resolve concrete read scopes without normalizing operator path spelling. |
| Resource silences | Kind/key, created time, optional expiry and a trimmed reason are retained. Expiry is exclusive: delivery resumes exactly at expiry. Disabled/silenced delivery does not mean the incident disappeared. | The planner and configured policy retain metadata and apply current in-app predicates. Qualify actual migration, genuine collection and current external-send policy checks; saved settings alone do not establish ordinary notification parity. |
| History and status | `alert_history.py` keeps 200 transitions under a 1-MiB read bound. `app.py::_default_overview_service` requests five recent recoveries. Daemon status separates latest delivery success/error from incident state. | Owned alert/notification API/UI and acknowledgment are registered. Production supervision, external failure views and installed retention must preserve deduplication and delivery ordering. |
| Manual delivery test | `MattermostIntegrationService.send_test` sends a distinct integration-test notification. The route requires login and CSRF; missing configuration returns 409 and transport failure 502. | An explicit authorized POST with a bounded, current destination. A manual test must not invent a healthy resource observation or reuse the narrow schedule-enablement grant as arbitrary credential authority. |
| Stack inbound events | Token-gated POST, 256-KiB route cap, quiet/verbose policies and test/noise suppression. Status exposes the ingest token to the authenticated operator. Re-provisioning preserves token/mode/source and writes mode 0600, while setting enabled true. | Current protected token/stack binding, durable admission identity and outbox rollback. Preserve the operator's ingest-URL copy flow through an authorized secret-reveal boundary; ordinary status/logs/outbox data must not expose credentials. Re-provisioning is a reviewed mutation, never a GET effect. |
| Held package updates | `pihealth_helper.py::_post_package_updates` reports newer held/critical or pinned entries after reconciliation. `_updates_webhook` prefers the enabled dedicated updates destination, then the alerts webhook. Notification failure does not fail the owned package reconciliation result. | Register genuine package outcome events and current delivery policy with durable receipts. Keep package reconciliation success separate from notification delivery; do not claim both succeeded from the reconciliation job alone. |
| Dedicated updates settings | `package-updates.json` contains version, enabled, webhook and channel. Provisioning selects `limeos-updates` and mode 0600. Disabling the dedicated destination still falls back to alerts. | Preserve destination priority or document an explicit reviewed replacement. A channel-specific disabled flag is not a global notification kill switch. Keep credential material out of ordinary settings views. |
| Container update preference | `update_service.py` defaults `notify_on_update` to true and persists it; the frontend exposes it. `_run_locked` does not read the setting, and this service has no notification sender. | Retain/migrate the preference and give supported delivery an explicit registered implementation/status. The saved flag is not evidence that the frozen app sent update notices. |

The verified outbound implementations above use Mattermost incoming webhooks;
manual tests share the health destination. Mattermost installation, bots and
model/report automation are not prerequisites for ordinary monitoring or in-app
notifications. The existing optional transport can still require preservation
when configured. `agent_automation/reporting.py` and the helper's report-credential
projection are a separate consumer of the alerts credential: changing/removing
that shared legacy secret needs an ownership/migration decision, not an inferred
transport retirement. This survey does not establish deployed channel use.

## Rules that need explicit migration handling

- The daemon coerces invalid integer strings to defaults, clamps zero/negative
  values to one and accepts an unbounded positive failure threshold. Captured
  values include a one-second poll and threshold 100. Rust's existing schedule
  minimum of 30 seconds and alert threshold maximum of 16 cannot silently accept
  those settings. Preserve the input for review or report an explicit unsupported
  migration; do not silently change its meaning.
- A valid empty policy overrides `LIMEOS_ALERT_REQUIRED_MOUNTS`. Treating empty
  and missing policy as equivalent would add unintended mount alerts.
- `_updates_webhook` falls back for disabled/missing/malformed dedicated
  configuration. A valid non-object JSON value instead raises `AttributeError`;
  the nightly caller reports a skipped update report. Typed Rust must return a
  bounded configuration failure rather than copying that exception behavior.
- The existing stack ingest service can answer `forwarded` with no poster call
  when its destination is absent. The prior handoff already requires an explicit
  blocked/unavailable result instead. A successful ingest response is not a
  verified delivery receipt.
- Source paths, webhook URLs and ingest tokens are not notification identities.
  Preserve exact source/configuration ownership and use central durable event
  keys. External ambiguous sends cannot acquire an exactly-once promise from
  these legacy synchronous helpers.

## Remaining acceptance

Accepted configured policy revisions now reach owner-scoped reservation,
leases, folding and current in-app publication. The original
[Store settlement](p04-b1-observation-settlement-handoff.md) default-only evidence
retains its historical scope; the later configured-policy handoff supplies the
new source. Protected actual-state migration and current external destination
references/revisions remain required before ordinary notification parity.

The protected collector adapter also needs an explicit mapping between legacy
name/path alert keys and concrete Rust read-resource identities. Review instance
replacement and resource-binding changes: an existing incident must neither be
silently rebound to a different object nor stranded behind an immutable old
binding. The current default-policy Store implementation does not qualify that
adapter.

Installed API/UI scenarios still need configured/missing/disabled destinations,
current grants/CSRF, token revocation, policy changes during collection/send,
delivery failure/ambiguity, incident-before-recovery ordering, queue/retention
bounds, restart and provider-disabled operation. P06 must inspect actual deployed
settings and preserve or explicitly retire each configured flow. No Pi access,
credential read, package effect, external send, channel retirement or cutover
qualification occurred here.
