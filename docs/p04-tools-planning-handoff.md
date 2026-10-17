# CopyParty tools planning handoff

Tested product source: `b265690a4b8dc5f112389880fe1048b91122d5f7`.
Stream B continuation on `engineer/p04-tools-planning`, based on network UI
handoff `53ec484` and combined main `7df3f75`. The
[design](plans/2026-10-17-tools-planning-design.md) and
[fixtures](../tests/fixtures/deployment/tools/README.md) define the supplied-fact
boundary. The [evidence packet](rewrite-evidence/p04/2026-10-17-tools-planning/README.md)
records the tested source and local checks. This is an intermediate RW-042
artifact; tools installation/configuration runtime, UI and phase acceptance remain
open. The first durable scheduling milestone still requires the central Store
contract and registration.

## Behavior supplied

`deployment::tools::Config` admits a literal absolute path up to 512 UTF-8 bytes
with nonempty, unambiguous components up to 255 bytes, and port 1–65535. Case and
internal spaces survive. Control characters, surrounding component whitespace,
root, trailing/repeated separators and dot traversal are refused. This validates
intent, without resolving paths, checking storage ownership or reserving a port.
It is not a systemd argument encoder; future templates must handle literal text
correctly. Debug output retains path length rather than path contents.

The supplied adapter distinguishes verified config-file absence, failed reads,
incomplete reads and complete bytes. Only absence produces the frozen defaults:
`/srv/copyparty`, port 3923. A valid stored object can omit fields and receives the
frozen key defaults. Nulls, arrays, duplicate/unknown keys, coercions, malformed
JSON and nonempty legacy `extra_args` produce typed unknown states. No diagnostic
retains raw text or arguments. The config byte cap is 16 KiB, checked before JSON
decoding. Nonempty legacy options, including `-v`, require an explicit migration
decision and typed support; they are not silently dropped or executable inputs.

The service adapter parses supplied complete, successful fixed-service
`LoadState`/`ActiveState` properties, capped at 512 bytes. It distinguishes active,
inactive, failed, activating/deactivating/reloading, missing, masked and load-error
states. Failed/nonterminal/timed-out/truncated captures and unsupported property
combinations supply unknown state. A masked but active service remains unknown,
without a stopped claim. These are synthetic captures; no systemd command was run
to qualify the proposed collector query.

`observe` projects configuration, legacy payload presence and service state
independently. Each retains its original time. Inclusive age 90 seconds is fresh;
older or future samples withhold values. Missing sources have null times/values.
Fresh observations of read errors remain explicit unknown values. A payload
`present` state means a legacy regular file, without package authenticity.
Service `active` does not establish readiness, reachability, user access or loaded
configuration. Applied configuration is always `not_established` in this slice.

`preview_config` supplies exact, deterministic path/port changes after fresh
usable observations. Missing config needs saving even when desired fields match
the defaults. Equal stored fields mean no save is needed, without an applied
result. Missing/unavailable/unsupported payloads, unreadable config and busy,
masked, unknown or unusable service states block the preview explicitly. Every
preview retains all six unresolved gates: trusted package/service identity,
explicit service account/access policy, storage ownership/port checks, current
authority/claims/locks, atomic save/apply/recovery and loaded-result verification.
It contains no command, unit text, download address, write artifact or receipt.

`DeploymentBackend::tools_status` defaults to unavailable. The proposed
`GET /api/v1/deployment/tools/copyparty` handler authenticates before accessing
the cache and caps the serialized response at 8 KiB. The backend must recheck
current grants. No install/configuration mutation is registered. Host/forwarded
headers cannot invent a service URL. Production route aggregation, Core/backend,
protected worker/cache, Store/schema/protocol, shared manifests/lock, frontend
shell and package/service activation remain unchanged.

## Central integration requirements

1. Accept/export the B domain and adapter modules and register a protected
   fixed-host collection under existing worker admission, deadlines, process/output
   limits and supervision. Resolve `copyparty.json` beneath the accepted protected
   config root, and the fixed legacy payload `/opt/copyparty/copyparty-sfx.py`.
   Distinguish successful absence from denied/failed lookups, symlinks, nonregular
   objects and incomplete reads. Supply complete config bytes only. No request
   chooses a target path, command or unit. The parser does not establish ownership.
2. Retain host/boot/config/service identity, protected read owners and original
   timestamps together in the shared cache. Collect only the fixed service
   `copyparty.service` with the selected LoadState/ActiveState properties; qualify
   real systemd results, exits, states and budget failures on a disposable host.
   Project `observe(now, facts)` against original samples on every cache read.
   GET cannot reset sample times, invoke collection or change files/services.
3. Implement `DeploymentBackend::tools_status` through the shared authenticated
   Core. Resolve current session/grants on cache access and withhold revoked or
   wrong-owner data. Merge `deployment::routes()` inside existing shared API
   middleware/limits before `with_state`; preserve security/no-store headers,
   deadlines and reader admission. The leaf-router tests do not qualify this
   production middleware or make the session check a permission lease.
4. Register schemas/generated contracts and add an authenticated tools UI using
   the wire fixture. Show separate component freshness and unknown states;
   invalidate retained data on logout, grant/host/boot changes and source expiry.
   Label legacy payload presence and saved configuration precisely. Future links
   need trusted endpoint/scheme/IPv6 metadata and reachability policy. This slice
   supplies neither links nor a browser-tested panel.
5. Before exposing configuration/install controls, accept a trusted package/source
   path, explicit service account and access policy. Do not reuse the frozen
   latest-release downloader, arbitrary extra arguments or implicit root unit.
   Resolve verified physical/storage dependencies and numeric ownership with A.
   Integrate review, current authority, complete claims/locks, before-images,
   atomic save/apply/recovery and verified loaded results through the central
   dispatcher. A matching saved config must never reconcile an interrupted apply
   as success. No parallel registry, approval or persistence is proposed here.
6. The existing qualification owner accepts installed API/UI permission/error
   cases, actual systemd observations, restart/grant/expiry behavior, supported
   configuration application and interruption/recovery. Installation is blocked
   until package provenance and access policy are accepted. Keep production Pis
   untouched while those gates are open.

## Local acceptance map and limits

| Preserved behavior or regression | Local evidence | Acceptance still required |
| --- | --- | --- |
| CopyParty defaults and stored overrides | 27 config fixtures, literal path/port admission | Protected config collection, API/UI and supported migration decisions |
| Status and helper/read failure distinctions | 20 property fixtures, command outcome/privacy tests, independent freshness | Actual fixed-service collector and installed status |
| Save before failed apply | Missing/equal stored preview cases always retain loaded-result/recovery gates | Central save/apply, restart and interruption recovery |
| Typed operation boundary / `SEC-002` allocation | Nonempty flags, unknown input and unverified installation never produce commands | Trusted installation, current authority and service confinement |
| Read-only transport / `API-001` allocation | Authentication precedes cache, backend grant loss, 405 mutation methods | Shared middleware, installed permission/CSRF and no-GET-effect checks |
| Service links / `UI-005` allocation | Request headers do not construct a URL | Trusted HTTPS/IPv6 service metadata and UI acceptance |

The combined leaf harness passes **499 Rust tests**: 321 domain, 137 supplied
adapter/host-policy, 35 API and six runner. This includes 38 new tools tests;
fixture rows are exercised inside those tests and are not extra test counts.
Strict all-target Clippy, Rustfmt and diff checks pass. Original domain/executor
libraries compile as dependencies; their full unit suites were not run. The
initial empty-array decoder failure and a maximum-path fixture length typo are
preserved with final successful checks. No frontend code changed or frontend
tests were run for this slice. No installed/Pi/ARM64, browser, performance, soak,
durable scheduling, recovery, defect closure, P04 completion or cutover credit is
claimed by this packet.
