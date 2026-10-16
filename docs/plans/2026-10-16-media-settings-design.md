# Servarr settings planning

Stream B follow-up to `1607167`. Preserve the frozen Sonarr/Radarr/Lidarr import,
recycle-bin, completed-download, naming and quality-profile intent through pure
typed previews and bounded supplied-response decoders. This needs no A changes
or central Store/runtime work.

Use the existing media service identity plus an exact container snapshot. A
complete observation represents the four required resource reads, with its
original oldest sample time. Reject malformed selected data; unavailable,
stale/future, incomplete, stopped and mismatched observations remain unknown and
propose no changes. Keep the inclusive 90-second bound. These ordinary DTOs do
not prove endpoint origin, acquisition completeness or protected ownership.

Correct the frozen wire mappings against primary API resource definitions:
`copyUsingHardlinks` and `recycleBin` belong to media management;
`enableCompletedDownloadHandling` belongs to download-client configuration;
naming uses `renameEpisodes`, `renameMovies` or `renameTracks` by application.
Sonarr/Radarr paths use API v3, Lidarr API v1. Closed move/hardlink and
standard/keep-original choices replace permissive arbitrary strings. Derive
the recycle bin as `.recycle` below the selected typed library container path;
actual storage, mounted bindings and numeric identity still require verification.

Only differing selected settings become typed changes. Preserve unknown service
configuration by leaving it out of the preview, never creating a partial PUT
body or copying secrets into a job. The future writer must recollect and merge
only reviewed fields into the exact owned resource preimage under common locks.
No HTTP request/body, write, receipt or execution authority is supplied.

Quality profiles retain the frozen exact-name lookup semantics. Report present,
missing and ambiguous matches explicitly. Missing/ambiguous requested profiles
block the entire draft. A present profile means available; the frozen lookup did
not assign a global default, so this preview does not invent one. The frozen
Lidarr `HD-1080p` request may therefore block instead of claiming configuration.
Readarr and Jellyfin settings are outside this explicit three-service subset.

Bound each response to 32 KiB before parsing, profiles to 16, names to 128 UTF-8
bytes, paths to 512 bytes, selected aggregate input text to 32 KiB and streaming
preview output to 64 KiB. Keep six source/endpoint, physical/ownership/hardlink,
current-configuration/field-preservation, authority/claims/locks, before-image/
recovery and verified loaded-result gates unresolved for every outcome.

Synthetic frozen/default and primary-schema fixtures cover app-specific fields,
separate resources, missing/duplicate/partial metadata, freshness, instance drift,
no-op reruns, profile ambiguity, path derivation and unknown credential sentinels.
Run domain and decoder tests sequentially in one isolated harness, strict Clippy,
Rustfmt and harness checks. Shared roots/manifests/production lock, schema/Store,
protocol, API/UI mounting and packages remain central. Installed service, browser,
ARM64/Pi and broad qualification remain with the existing test owner.
