# Cached network-group read surface

Continue Stream B from `31b3b68` with the cached network-group API and a mount-ready
frontend panel. The existing network domain and supplied-inspect decoder already
describe configured references, providers, members, stopped/one-off containers,
health distinctions and orphan hints. This slice exposes those facts without
collecting new evidence or triggering a probe.

Add `DeploymentBackend::network_inventory` with an unavailable default and an
authenticated `GET /api/v1/deployment/networks` handler. Resolve the current
session before cache access and require the actual backend to resolve current
read grants in its cache transaction. Check the 64 KiB serialized response bound.
GET performs no Engine read, HTTP probe, process launch or write; mutation
methods have no route. The integrator still supplies the protected cache/backend,
production exports/routes, middleware and generated contracts.

Prefer a dedicated bounded client/controller/view over extending stack discovery
DTOs or building an active network test. Decode the existing version-one Rust
wire shape, construct only admitted fields and reject inconsistent source age,
counts, duplicate/conflicting identities, invalid references and group/member/hint
associations. Enforce the domain's 16-container, ID/name/path/project bounds and a
64 KiB streamed response ceiling. Unknown/error observations require null counts
and no groups or members. Only a fresh complete empty cohort can display zero.

Use the dashboard's existing utilitarian cards, badges, source rows and table
styles. Show provider identity, configured health, actual members and one-offs,
reference diagnostics and unverified hints separately. "Provider running" and
"No configured health check" never become "VPN safe" or a positive namespace
result. Egress comparisons retain their separate observation contract and are
not joined to topology rows by name or inferred lifetime.

The panel owns one principal/grant lifetime. Coalesce reads, abort and ignore
abandoned work, refresh on focus/reconnect or explicit request and stop on access
loss. Preserve the original sample deadline with the inclusive 90-second Unix
second bound, including early timer handling and clock rollback. Clear retained
rows on expiry, read failure, 401/403 and disposal. HTTP access status takes
precedence over malformed/private error bodies. No polling or automatic POST
retry is introduced. All fetches use the fixed same-origin GET route with a
deadline and bounded fatal UTF-8/JSON parsing.

Bind a synthetic wire fixture to Rust `observe` output and consume the same file
in frontend tests. Cover authenticated cache-only access, forbidden/missing
session, default-unavailable behavior, report limits, orphan/health/one-off UI,
invalid wire, source-age/count/association contradictions, failed reads, streamed
body limits, aborted/late work and principal/grant/session lifetime handling.
Run combined Rust/Clippy and pinned frontend type/unit checks sequentially with
repository disk scratch. Use a local fixture preview for visual inspection when
available. Store/schema/protocol, shared shell/manifest/lock, installed probes,
service activation and production Pis remain outside this slice.
