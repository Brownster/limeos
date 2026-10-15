# Media library seeding previews

Engineer B, 15 October 2026. Base catalog `c712486`. A's mount work and the
central Store/runtime work remain uncommitted. No A-owned path is needed for
this slice: consume the existing immutable StorageContract rather than create
a second storage or ownership model. Registration remains a central dependency.

Deliver pure Servarr root-folder and Jellyfin library previews with bounded,
selective response decoders. Preserve frozen Sonarr/TV, Radarr/movies,
Lidarr/music and Jellyfin's five library defaults. Download clients, indexer
applications, media-management/naming/quality defaults, local config seeding,
provisioning, quickstart execution and activity remain separate parity rows.

Derive each library's host path from the validated storage contract, then derive
its container path from complete supplied bind declarations. Support existing
`/tv`, `/movies`, `/music`, `/media/<kind>` layouts and canonical `/data/media`
layouts without changing their bytes or case. Refuse missing, ambiguous, shadowed
or read-only Servarr bindings; Jellyfin may read a read-only media mount. Preserve
numeric UID/GID and require an exact match to managed media identity. Reject
overlapping media/download roots, download-backed or reserved download container
paths, traversal, unresolved template text and unsupported mount uncertainty.
Path declarations prove neither mounted filesystem identity nor hardlinks.

Use an explicit stack/service/Engine/project identity with original sample time
and completeness. Missing, partial, stale, future or mismatched observations
produce unknown rather than an empty successful seed. Complete fresh zero rows
can propose additions. Deduplicate by exact normalized path for root folders,
and case-insensitive name/type with exact paths for Jellyfin, preserving path
case. Duplicate keys/IDs or conflicting existing libraries require review rather
than silently selecting one or adding another same-name library.

The frozen seed removes every root under `/downloads` before adding a canonical
root. Preserve these as removal *candidates* in the preview, including actual
download bindings and `/data/downloads`; never delete anything here. Existing
library references, current grants, backup/recovery and an explicit review are
required before changing service configuration. Unrelated roots/libraries stay
untouched. All preview gates remain unresolved, even for an unchanged result.

The decoders accept only supplied successful-response bytes: arrays of object-
shaped selected metadata, bounded before parsing. Consume root ID/path and
Jellyfin name/type/locations only. Skip secrets and unrelated payloads; reject
duplicate consumed aliases, malformed objects/fields and oversize arrays before
retaining an unbounded collection. No HTTP client, credentials, process, disk
read/write, opaque owner, approval, receipt or execution adapter is added.

Bounds: 32 KiB per response, 16 root/library records, 8 paths per library,
8 mount declarations, 512 bytes per literal path, 128 per library name/type,
32 KiB aggregate admitted text and 64 KiB streaming summary. B2's project/path/
file identity limits apply. Fixture ports cover the three existing storage
profiles, legacy/canonical mounts, numeric identity, path/hardlink boundaries,
reruns, unsafe-root candidates, duplicate/conflicting libraries, unknown sources,
privacy and bounds. One local Cargo job; no broad qualification or host effects.

Reference source is frozen pi-health `80593b29443ea9b81eb60ed4e5c3c3ac7a236204`:
media_seed_service.py, media_layout.py, storage_compatibility.py, relevant catalog
YAML and tests. Read/hash reference files only; do not import or execute them.
