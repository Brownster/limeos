# Prowlarr application registration planning

Continue Stream B from `f0fd86c` with pure Prowlarr registration previews and a
bounded supplied-response decoder. Port the existing Sonarr/Radarr/Lidarr
registration intent, without creating credential-bearing payloads, collecting
protected sources or using central Store/locks/recovery before registration.

The target is explicitly Prowlarr, with exact stack/Engine/project/service and
container/image/start identity. Reuse the existing internal media scope syntax
validator through a narrow helper; do not introduce another authority registry.
A complete observation joins registered service declarations and `/api/v1/applications`
metadata with the oldest original timestamp. Missing, incomplete, stale/future,
stopped or mismatched observations propose nothing. A declaration proves no
running peer, endpoint or connectivity.

Requests name explicit case-sensitive peer services, desired category sets and
tags. The desired sync level is the frozen `fullSync`. Support one request per
Sonarr/Radarr/Lidarr application, at most three distinct services. Use an explicit
conservative category subset: TV 5000–5999 for Sonarr, movies 2000–2999 for Radarr,
music 3000–3999 for Lidarr. Sonarr also requires an explicit anime TV category set;
other apps have no anime field. Refuse the frozen TV categories for movies/music
instead of silently correcting operator intent or claiming success. No new
defaults or reset policy are inferred from current upstream defaults.

Match implementation without ASCII case sensitivity. A uniquely matching
implementation can retain an operator's name; adding another canonical name on
rename would duplicate it. Multiple instances are ambiguous without actual
endpoint bindings and block. Missing selected metadata, conflicting categories,
tags or sync level, and canonical name collisions block rather than modifying
operator configuration. Compare category/tag sets without order sensitivity;
duplicates refuse. If any row blocks, suppress all addition candidates. No-op
means selected metadata matches, not successful service configuration.

Candidates are incomplete review data with app/config contract, full-sync intent
and category/tag policy; no host, port, URL, key, config-file path or POST body.
The writer must establish both Prowlarr-to-app and app-to-Prowlarr connectivity and
current protected credentials. Prefer this selected-data boundary over carrying
arbitrary provider objects or inventing executable request authority.

Bounds: raw response 32 KiB; 16 applications; 64 fields per application; 16 IDs per
selected category/tag set; selected text 128 UTF-8 bytes; 16 declarations; 32 KiB
aggregate text; 64 KiB streamed report; inclusive 90-second age. Missing/null
selected settings remain unknown; duplicates, wrong types and over-limit data
refuse. Omit unknown fields and credentials from selected DTOs/previews. Keep six
source/current-instance, peer/network/credential, schema/configuration,
authority/locks, before-image/recovery and independently verified-result gates.

Use frozen intent plus synthetic native metadata fixtures for all three apps.
Test reruns, rename/duplicate/name collisions, category defects, order/unknown
settings, declaration/identity/freshness failures, bounds and secret exclusion.
Run unfiltered domain/decoder tests sequentially in one standalone harness,
Clippy, Rustfmt and helper checks. Shared roots/manifests/production lock,
Store/schema/protocol, API/UI mounting, packages/publication and installed/Pi
qualification remain with their owners. This slice cannot complete RW-042/P04.
