# Download-client registration planning handoff

Engineer B, 16 October 2026. Branch `engineer/p04-media-download-clients`, base
`66241ef`. Tested product source `d457fc821a430b69dfdfeb0464a8ba29e7d376dc`.
This continues the settings/local-config/media/catalog lineage. Other B branches
retain separate source identities; no A or central runtime changes were needed.

## Delivered

`domain::deployment::download_clients::preview` ports the frozen Transmission and
SABnzbd registration intent for Sonarr, Radarr and Lidarr. Its executor-container
sibling decodes supplied download-client JSON. It performs no acquisition,
credential read, HTTP, command or write.

- Use native `tvCategory`, `movieCategory` or `musicCategory` and API v3 for
  Sonarr/Radarr, v1 for Lidarr. Generic top-level/field `category` does not replace
  the native field. Missing/null native values remain unknown, blocking additions
  for that implementation. Empty observed strings remain distinct known values.
- Match implementation without ASCII case sensitivity and category exactly.
  Duplicate matching registrations, unknown categories, name collisions, disabled
  clients or differing removal policy block the draft. Preserve unrelated rows;
  never silently update operator configuration. Any blocked row suppresses all
  additions, while other rows retain their diagnostic decisions.
- Requests explicitly identify case-sensitive Compose peer service names. An
  absent peer is `not_declared`, distinct from unknown source. A declaration proves
  neither a running service nor a reachable endpoint. Present means selected
  metadata matches; credentials, endpoint, tags, protocol, priority, connectivity
  and actual provider behavior remain unverified.
- Missing registrations become incomplete typed review candidates with provider,
  name/config contract, category field and removal intent. They contain no host,
  port, authentication or arbitrary provider body. They are **not POST bodies**;
  the frozen payload omitted required connection material, which remains pending.
  The intended new client is enabled; the future writer must express that
  explicitly in its schema-validated body.
  At most one request per provider and two distinct peer services are supported.
- The complete observation joins current registered declarations and API rows for
  the exact settings target: stack/Engine, ordered project files and directory,
  service/application, full container/image/start snapshot. Keep the oldest
  original timestamp across both sources. Missing target declaration or partial
  data is incomplete; unavailable/stopped/mismatched/future/stale data proposes
  nothing. Age 90 seconds is admitted. Decoded category-field identity also refuses
  relabeling Sonarr metadata as Radarr/Lidarr metadata.

Bounds: raw body 32 KiB before parsing; 16 registrations; 64 fields per row;
128 UTF-8 bytes per selected string; 16 declared service names of at most 64 bytes;
32 KiB aggregate selected input text; 64 KiB streamed output. Desired Transmission
categories use nonempty ASCII letters/hyphens with an optional leading dot;
SABnzbd categories use bounded literal text. This is a conservative subset.
Duplicate consumed fields, casing aliases, wrong types, invalid IDs and duplicate
IDs refuse. Unknown provider fields are omitted from selected DTOs and previews;
names/categories remain intentionally selected private data. No raw-byte custody
or zeroization is claimed.

## Integration and remaining work

Register the leaves centrally after the preceding B lineage. Existing dependencies
suffice; shared roots, manifests/production lock, Store/schema, protocol, routes,
frontend shell, packages and activation are unchanged. The standalone harness
includes dormant modules only for local checks.

All six gates remain unresolved, even for no additions: protected sources/current
instance; verified peer network/credentials/storage; installed schema/full
configuration; current authority/claims/locks; before-image/recovery; independently
verified loaded result. Resolve endpoints through current Compose/VPN namespace
evidence, without guessing localhost or using a caller's URL/credential string as
authority. Validate provider schema and connectivity under the approved runtime.
Recollect before dispatch under shared deployment/storage/backup serialization,
preserve operator rows/fields, and reconcile uncertain creation by current
identity rather than blindly retrying. Supply durable review/job/status API/UI
through the central path; GET must cause no effect.

Prowlarr application registration, complete media quickstart, broader source/VPN
support and registered execution/API/UI acceptance remain open. B1 still awaits
the accepted durable Store contract/registration. RW-042 and P04 remain incomplete.

## Evidence

[Packet](rewrite-evidence/p04/2026-10-16-download-clients/README.md): **174 passing
local tests** — 145 domain and 29 decoder. The slice adds 18 domain and 9 decoder
tests; the previous 147 remain included. Strict all-target Clippy, Rustfmt, Ruff
and generated-harness byte checks pass. Product bytes were committed after these
checks without intervening edits. The first test run passed; no failed-run or
live Python equivalence result is claimed. Frozen files were read/hashed only.

No protected collection, actual service/version/schema, network/credential,
write/recovery, authenticated API/UI/browser, package/ARM64/Pi, footprint,
performance or soak qualification ran. Main and production hosts remain unchanged;
the branch is local, unmerged and unpublished.

Primary provider definitions, inspected 16 October 2026 on unpinned `develop`:
Sonarr [Transmission](https://raw.githubusercontent.com/Sonarr/Sonarr/develop/src/NzbDrone.Core/Download/Clients/Transmission/TransmissionSettings.cs)
and [SABnzbd](https://raw.githubusercontent.com/Sonarr/Sonarr/develop/src/NzbDrone.Core/Download/Clients/Sabnzbd/SabnzbdSettings.cs);
Radarr [Transmission](https://raw.githubusercontent.com/Radarr/Radarr/develop/src/NzbDrone.Core/Download/Clients/Transmission/TransmissionSettings.cs)
and [SABnzbd](https://raw.githubusercontent.com/Radarr/Radarr/develop/src/NzbDrone.Core/Download/Clients/Sabnzbd/SabnzbdSettings.cs);
Lidarr [Transmission](https://raw.githubusercontent.com/Lidarr/Lidarr/develop/src/NzbDrone.Core/Download/Clients/Transmission/TransmissionSettings.cs)
and [SABnzbd](https://raw.githubusercontent.com/Lidarr/Lidarr/develop/src/NzbDrone.Core/Download/Clients/Sabnzbd/SabnzbdSettings.cs).
These establish the corrected category mapping, without qualifying an installed
version. The resource definitions expose the selected enable/removal flags in
[Sonarr](https://raw.githubusercontent.com/Sonarr/Sonarr/develop/src/Sonarr.Api.V3/DownloadClient/DownloadClientResource.cs),
[Radarr](https://raw.githubusercontent.com/Radarr/Radarr/develop/src/Radarr.Api.V3/DownloadClient/DownloadClientResource.cs)
and [Lidarr](https://raw.githubusercontent.com/Lidarr/Lidarr/develop/src/Lidarr.Api.V1/DownloadClient/DownloadClientResource.cs).
