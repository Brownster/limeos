# Download-client registration planning

Continue Stream B from `66241ef` with the frozen Transmission/SABnzbd registration
intent in Sonarr, Radarr and Lidarr. Keep Prowlarr application registration for a
separate slice. This work needs no shared Store contract or Stream A changes.

Use a pure preview and a supplied-response decoder. A complete observation joins
the exact existing media-settings target, current registered Compose service
names and download-client rows, retaining the oldest original sample time.
Unavailable, partial, stopped, future, stale or instance-mismatched observations
propose nothing. A declaration means declared, not running or reachable.

Requests explicitly name each provider's Compose service and category plus the
frozen removal flags. Support one request per provider, at most two. Match the
implementation without ASCII case sensitivity and the category exactly. Decode
native `tvCategory`, `movieCategory` or `musicCategory` fields; generic `category`
does not substitute. Missing/null native category remains unknown, blocking an
addition for that implementation. Duplicate selected fields/IDs refuse; duplicate
matching registrations, name collisions, disabled clients and differing removal
policy block instead of silently accepting or overwriting operator settings.

Present means the selected registration metadata matches, without connection or
credential verification. A missing declared peer is explicitly skipped. Missing
registrations yield typed addition candidates with the corrected category field;
these are incomplete review data, never POST bodies. If any row blocks, suppress
all candidates. Sort requests/results deterministically and preserve unrelated
registrations. Never infer host/port/credentials from a provider name or localhost.

Alternatives considered: retaining arbitrary credential-bearing provider objects
would unnecessarily carry secrets into previews; producing executable creation
bodies would require the central source/authority/network/recovery boundaries.
Selected metadata and honest unresolved gates provide an independently testable
port now, while keeping those boundaries with their owners.

Bound raw response bytes to 32 KiB, registrations to 16, fields per row to 64,
selected strings to 128 UTF-8 bytes, declared services to 16, selected aggregate
input text to 32 KiB and streamed output to 64 KiB. Reuse exact target validation
and the inclusive 90-second age bound. Keep six acquisition, peer/network/secret/
storage, schema/configuration, authority/locks, before-image/recovery and verified
result gates unresolved for every outcome.

Use synthetic native-schema fixtures for all three apps plus frozen defaults.
Test rerun matching, category/implementation case, unknown category, duplicates,
policy/name conflicts, absent declarations, original timestamps, identity drift,
raw/decoded/report limits and secret exclusion. Run unfiltered domain/decoder
checks in one standalone harness, strict Clippy, Rustfmt and helper checks.
Shared root exports/manifests/lock, Store/schema/protocol, collectors/writers,
API/UI mounting, packages, publication and Pi qualification remain separate.
