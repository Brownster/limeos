# Prowlarr application registration planning handoff

Engineer B, 16 October 2026. Branch `engineer/p04-media-indexer-apps`, base
`f0fd86c`. Tested product source `3c6669a8aa6a8f7669b3a4ee6654ccf8b6a3600b`.
This continues the download-client/settings/local-config/media/catalog lineage;
other B deliveries retain separate source identities. No A or central runtime
change was needed.

## Delivered

`domain::deployment::indexer_apps::preview` accepts an explicit Prowlarr target,
typed Sonarr/Radarr/Lidarr requests and optional selected application metadata.
The executor-container sibling decodes supplied `/api/v1/applications` JSON.
Neither performs source acquisition, HTTP, credential/file access, commands or
effects. The narrow `media::scope_valid` helper shares the existing internal scope
syntax rules; existing media identity validation retains the same conditions.
Shared crate roots/manifests/production lock and registration are unchanged.

- The requested sync level is the frozen `fullSync`. Requests explicitly name
  case-sensitive Compose peer services, category sets and tag sets. At most three
  distinct services and one request per application type are supported.
- Desired category admission is an explicit conservative subset: Sonarr TV
  5000–5999, Radarr movies 2000–2999, Lidarr music 3000–3999. Sonarr requires an
  explicit anime TV set, which may be empty; other apps have no anime field.
  This range check proves neither that a category exists nor installed capability.
  TV-only movie/music requests return `category_scope`, rather than silently
  changing intent. No defaults or reset policy are inferred from upstream.
- Selected existing categories can be outside that desired subset so a bad
  frozen configuration reaches an explicit policy conflict. The synthetic frozen
  TV-for-all fixture matches Sonarr metadata but blocks Radarr and Lidarr against
  the example movie/music requests. It creates no correction or update body.
- Match implementation without ASCII case sensitivity. A unique registration
  keeps an operator's chosen name, avoiding a duplicate canonical-name addition
  after rename. Multiple same-implementation instances block as ambiguous until
  actual endpoint/peer bindings can distinguish them. Canonical name collisions
  with another implementation also block.
- Missing/null categories, tags, sync level or Sonarr anime metadata remain
  incomplete. Unknown sync-level strings remain unknown. Known differing sync
  level/categories/anime/tags block instead of overwriting operator settings.
  Sets compare without order sensitivity; duplicate IDs refuse. Results and
  candidate policies are ordered deterministically. Any blocked row suppresses
  all candidates while retaining the other diagnostic row decisions.
- `matching_metadata` means only those selected values match. It proves no URL,
  credential, connectivity, indexer capability, health or verified configuration.
  Absent declared peers are `not_declared`; declaration proves no running peer.
  Candidates are incomplete review data with app/name/config contract, full-sync
  intent and category/tag policy. They contain no URL, key, host, port, config-file
  path or API body. Neither endpoint direction is guessed from loopback or names.

The target includes stack/Engine, project name/directory/ordered files, service,
full container/image/start snapshot. A complete observation joins registered
declarations and application rows with the original oldest timestamp. Missing
target declaration or partial data is incomplete. Unavailable/stopped/mismatched/
future/stale observations propose nothing; age 90 seconds is still admitted.
These ordinary DTOs prove neither protected origin nor acquisition completeness.

Bounds: raw body 32 KiB before parsing; 16 applications; 64 fields per row; 16 IDs
per selected category/tag set; selected names/implementation text 128 UTF-8 bytes;
16 declared service names of at most 64 bytes; 32 KiB aggregate selected text;
64 KiB streamed output. Required shapes, numeric IDs, duplicate consumed fields/
casing aliases, duplicate IDs, wrong types and limits refuse. Unknown fields are
omitted; anime fields on non-Sonarr apps are unselected. Names and desired policy
remain private data. No raw-byte custody or zeroization is claimed.

## Integration and remaining work

Register leaves centrally after the preceding B lineage. All six gates remain
unresolved even for no additions: protected sources/current instance; verified
peer bindings/network/credentials; installed schema/full configuration; current
authority/claims/locks; before-image/recovery; independently verified loaded result.

Qualify the installed application/schema and collect bounded, status-checked
responses from protected owners. Bind each peer and both Prowlarr-to-app and
app-to-Prowlarr endpoints through current Compose/VPN namespace evidence. Obtain
protected current credentials and explicit category/tag intent. The frozen
loopback assumption and missing callback URL are not carried into candidates.
An authenticated runtime must build a complete schema-validated body under shared
deployment/storage/backup serialization, bind reviewed authority/preimages, retain
operator configuration and reconcile uncertain creation without blind replay.
Register durable review/job/status API/UI centrally; GET must cause no effect.

Complete media quickstart orchestration, protected source/network support and
installed API/UI acceptance remain open. B1 still awaits the accepted durable
Store contract/registration. RW-042 and P04 remain incomplete; no feature is retired.

## Evidence

[Packet](rewrite-evidence/p04/2026-10-16-indexer-apps/README.md): **202 passing
local Rust tests** — 163 domain and 39 decoder. This adds 18 domain and 10 decoder
tests; the previous 174 remain included. Strict all-target Clippy, Rustfmt, Ruff
and generated-harness byte checks pass. Product bytes were committed after checks
without intervening product edits. First test/lint runs passed. Frozen files were
read/hashed only, never imported/executed; fixtures are synthetic with fake keys.

No protected collector/writer, actual application/version/schema, network or
credential verification, write/restart/recovery, authenticated API/UI/browser,
package/ARM64/Pi, performance, footprint or soak qualification ran. Main and
production hosts are unchanged; the branch remains local, unmerged and unpublished.

Primary Prowlarr definitions, inspected on unpinned `develop`, 16 October 2026:
[Sonarr settings](https://raw.githubusercontent.com/Prowlarr/Prowlarr/develop/src/NzbDrone.Core/Applications/Sonarr/SonarrSettings.cs),
[Radarr settings](https://raw.githubusercontent.com/Prowlarr/Prowlarr/develop/src/NzbDrone.Core/Applications/Radarr/RadarrSettings.cs),
[Lidarr settings](https://raw.githubusercontent.com/Prowlarr/Prowlarr/develop/src/NzbDrone.Core/Applications/Lidarr/LidarrSettings.cs),
[application resource](https://raw.githubusercontent.com/Prowlarr/Prowlarr/develop/src/Prowlarr.Api.V1/Applications/ApplicationResource.cs)
and [sync levels](https://raw.githubusercontent.com/Prowlarr/Prowlarr/develop/src/NzbDrone.Core/Applications/ApplicationSyncLevel.cs).
These support the native field/sync-level mapping and app-specific category
families; they do not establish an installed version or approval of a new policy.
