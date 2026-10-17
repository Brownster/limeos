# Cached CopyParty status UI design

Stream B, based on `f8aec4a`. Continue the tools planner/API handoff with a
bounded frontend client, observation-lifetime controller and read-only panel.
The proposed wire remains the Rust `deployment::tools::ToolsReport` and its
checked `tests/fixtures/deployment/tools/wire.json`. No Rust contract changes.

Use the existing industrial dashboard styling: dark background, lime accent,
plain status labels and three separate observation rows. Keep configuration,
legacy payload presence and service state readable independently. Show original
sample times, explicit defaults versus stored values, unknown causes, and the
unverified loaded-configuration result. Provide cached refresh only. No service
links, install/configuration controls or user-facing implementation instructions.

The client selects known fields, enforces the version-one enums, timestamps,
component relationships, literal path/port rules and 8-KiB wire limit. Unknown
private metadata is discarded. Request only the fixed same-origin cached GET,
with no-store, a 12-second abort deadline, streamed bounds and strict UTF-8/JSON.
HTTP 401/403 takes precedence over bodies; raw transport/error text is never
displayed. Validate shape only; protected collection provenance remains central.

Retain original per-component deadlines. At inclusive age 90 seconds values are
current; at age 91 remove only expired component values. Never upgrade stale or
future supplied facts as time passes. Clock rollback before report generation,
invalid clocks, read failures and access loss clear retained data. Expiry timers
perform local invalidation without polling. Coalesce reads and cancel/ignore
abandoned work. Reads/renders/timers check time; continuous clock-change detection
is not claimed.

Mounting requires principal, current grant revision and an opaque host/boot/cache
lifetime from the integrator. Missing ownership context withholds the panel and
launches no read. Each change creates a new controller, removing previous data
before effects run; logout unmounts it. The identifier scopes the UI lifetime and
does not prove server provenance or add an authority registry. The backend still
rechecks grants and protected evidence at cache access.

Using one whole-report expiry would discard still-current components or display
stale ones. Sharing network topology's controller would preserve that incorrect
cohort model. A dedicated small controller fits the existing per-source wire;
shared transport refactoring can follow independently. Registering production
routes, contracts, host context or shell here would cross the integrator's scope.

Use synthetic wire/markup/controller tests with deterministic clocks, abort and
late-result regressions, pinned type checking and a standalone fixture preview
build. Run suites sequentially in repository disk caches. Attempt visual review
only through an available browser surface; record any unavailable surface without
claiming visual acceptance. Existing Rust results retain their source identity;
this frontend-only increment adds no Rust, installed, hardware, performance,
recovery, defect-closure or P04-completion credit.
