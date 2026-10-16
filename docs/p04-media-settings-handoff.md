# Servarr settings planning handoff

Engineer B, 16 October 2026. Branch `engineer/p04-media-settings`, base `1607167`.
Tested product source `2e283f76a80ba47e3cfab4c495c0d629712d97ca`. This continues
the local-config/media/catalog branch lineage; other B branches retain separate
source identities. No Stream A change or central runtime dependency was needed.

## Delivered

`domain::deployment::media_settings::preview` accepts a Sonarr, Radarr or Lidarr
target, typed intent and optional selected settings. It returns a bounded typed
preview with resource IDs, field names, before/after values and unresolved gates.
The executor-container sibling decodes supplied JSON bytes for media management,
download-client configuration, naming and quality profiles. It performs no HTTP,
credential read, file access, command or effect.

| Frozen intent | Typed behavior and corrected mapping |
|---|---|
| Move/hardlink preference | `copyUsingHardlinks` on media management, rather than ignored `useHardlinks`. Move selects false; hardlink selects true. |
| Recycle bin | Derive `.recycle` below the selected typed library container path; retain path case and interior spaces. |
| Completed-download handling | `enableCompletedDownloadHandling` on download-client configuration, with its own resource ID. |
| Standard/keep-original naming | Sonarr `renameEpisodes`, Radarr `renameMovies`, Lidarr `renameTracks`; `replaceIllegalCharacters` remains true as in the frozen intent. |
| Quality profile | Exact, case-sensitive name availability check. Present is availability, without a default assignment; missing or ambiguous blocks every proposed change. |
| Application paths | Sonarr/Radarr fixed API v3 read paths; Lidarr API v1. Readarr and Jellyfin settings are outside this subset. |

An enabled custom import script blocks the draft without proposing to disable
operator configuration. `useScriptImport` must be a known boolean. Disabling
hardlinks does not guarantee a filesystem move; active download handling can
still copy. A hardlink preference proves no successful hardlink or shared device.
The synthetic Lidarr fixture has audio profiles, so the frozen `HD-1080p` request
explicitly blocks instead of reporting a fictitious successful default setting.

Only differences appear in a stable resource/field order. A fresh observation of
the desired values returns unchanged with all six gates unresolved. Unknown
source, stopped target, exact instance mismatch, incomplete response cohort,
future timestamp or age above 90 seconds returns unknown with no changes.
The original oldest sample time is retained, and age 90 is still admitted.
Malformed supplied selected data refuses rather than becoming defaults.

Identity includes stack, Engine, ordered project config files and working
directory, project/service/application and full container/image/start snapshot.
These ordinary DTOs establish neither origin nor acquisition completeness.
The caller must supply the original oldest timestamp across all four resources;
copying a newer cache/read time would invalidate that contract.

Bounds: 32 KiB per raw response before parsing; 16 profiles; 128 UTF-8 bytes per
profile name; 512 bytes per path; 32 KiB aggregate selected input text; 64 KiB
streamed preview. Required fields, object shapes, numeric IDs, duplicate selected
fields/IDs, text bounds and app-specific rename flags are checked. Duplicate
profile names survive decoding so the domain can report ambiguity. Unknown
configuration and credential fields are omitted. Existing recycle paths and
profile names are intentionally selected private data and may appear in a preview.

## Integration and remaining parity

1. Export/register these leaves after the preceding B lineage. Existing serde
   dependencies suffice. The local harness includes dormant modules without
   changing shared crate roots/manifests/production lock. The previous namespace
   test's fixture path is now source-relative so nested harnesses can compile.
   Existing decoder helper visibility changes only within the deployment parent;
   its behavior is unchanged and its ten library decoder tests still pass.
2. Qualify the installed application version/schema. The primary upstream
   resource definitions below were inspected on 16 October 2026 on unpinned
   `develop`; they establish the corrected mapping, not installed compatibility.
   Acquire all responses through a protected, bounded, status-checked collector
   tied to the exact current service/endpoint/instance. These byte decoders do not
   establish HTTP success, endpoint trust, credential custody or original timing.
3. Bind the selected library to the existing storage contract, actual mounts and
   numeric ownership. Built-in download roots are refused as library paths;
   custom download-directory overlap, writable mounted state and actual hardlink
   behavior remain physical verification work.
4. Recollect each full current resource under common deployment/storage/backup
   claims and locks. Preserve unknown fields and the exact resource ID in the
   protected writer. **A preview is not a PUT body**: sending only these selected
   fields could reset unrelated configuration. Bind preimages and reviewed diffs
   to current authority, durable recovery and independent loaded-result checks.
   No write method, receipt or execution approval is supplied here.
5. All six gates remain unresolved: protected source/endpoint; verified library
   mounts/ownership/hardlinks; current configuration/field preservation; current
   authority/claims/locks; before-image/recovery; verified loaded service result.
   Authenticated review/job/status API/UI mounting remains central. GET must
   cause no service effect. Download-client/indexer registration, complete media
   quickstart and broader gateway/source support remain open. B1 still needs the
   accepted durable Store contract/registration. RW-042 and P04 remain incomplete.

## Evidence

[Packet](rewrite-evidence/p04/2026-10-16-media-settings/README.md): **147 passing
local Rust tests** — 127 domain and 20 decoder. This adds 18 settings-domain and
10 settings-decoder tests; the remaining 119 are prior tests included in this
harness. Strict all-target Clippy, Rustfmt, Ruff and generated-harness byte checks
pass. Product bytes were committed after these checks without intervening edits.

The initial compile failure is retained: an older namespace fixture path assumed
a one-package harness. The source-relative correction allowed the nested harness
to compile. The null-first duplicate rename-field regression passed on its first
run; no hypothetical serde fix was added or credited. Frozen sources were read
and hashed only, never imported or executed; fixtures contain fake sentinels.

No protected collector/writer, field-preserving merge, authenticated API/UI,
browser, actual service/network/configuration, stop/write/restart/recovery,
package/ARM64/Pi, footprint, performance or soak qualification ran. Main and
production hosts are unchanged. This branch is local, unmerged and unpublished.

Primary resource definitions: Sonarr [management](https://raw.githubusercontent.com/Sonarr/Sonarr/develop/src/Sonarr.Api.V3/Config/MediaManagementConfigResource.cs),
[download handling](https://raw.githubusercontent.com/Sonarr/Sonarr/develop/src/Sonarr.Api.V3/Config/DownloadClientConfigResource.cs),
[naming](https://raw.githubusercontent.com/Sonarr/Sonarr/develop/src/Sonarr.Api.V3/Config/NamingConfigResource.cs);
Radarr [management](https://raw.githubusercontent.com/Radarr/Radarr/develop/src/Radarr.Api.V3/Config/MediaManagementConfigResource.cs),
[download handling](https://raw.githubusercontent.com/Radarr/Radarr/develop/src/Radarr.Api.V3/Config/DownloadClientConfigResource.cs),
[naming](https://raw.githubusercontent.com/Radarr/Radarr/develop/src/Radarr.Api.V3/Config/NamingConfigResource.cs);
Lidarr [management](https://raw.githubusercontent.com/Lidarr/Lidarr/develop/src/Lidarr.Api.V1/Config/MediaManagementConfigResource.cs),
[download handling](https://raw.githubusercontent.com/Lidarr/Lidarr/develop/src/Lidarr.Api.V1/Config/DownloadClientConfigResource.cs),
[naming](https://raw.githubusercontent.com/Lidarr/Lidarr/develop/src/Lidarr.Api.V1/Config/NamingConfigResource.cs).
