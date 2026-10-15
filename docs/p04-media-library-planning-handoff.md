# Media library planning handoff

Engineer B, 15 October 2026. Branch `engineer/p04-media-library-planning`, based
on catalog `c712486` (and B2). Tested code:
`5379d57bb2fd2b94e319ab17dc251405cc6fa0c1`.

## Delivered behavior

`domain::deployment::media::preview` proposes Sonarr/TV, Radarr/movies,
Lidarr/music and Jellyfin Movies/TV/Music/Books/Audiobooks library changes. It
consumes the existing `StorageContract`, declared numeric media identity,
complete resolved mount declarations and a scoped ordinary observation. It
neither replaces storage rules nor proves physical state or authority.

- Derive host paths from the contract and container paths from bind declarations.
  Existing `/tv`, `/movies`, `/music`, `/media/<kind>` and canonical `/data/media`
  layouts work without rewriting them. Preserve host/container path case and
  numeric UID/GID. Resolved PUID/PGID/catalog identity must equal the contract;
  this is not proof of the running process user or filesystem owner.
- Block missing/ambiguous exposures, named-volume or nested-mount shadowing,
  overlapping media/download roots, unresolved template paths and read-only
  Servarr library bindings. Jellyfin may use a read-only library bind. These
  conservative declaration checks do not establish writable mounted state,
  filesystem identity, symlink safety, path existence or hardlink support.
- Match original observation to exact stack, service, application, Engine and
  project/directory/ordered-files identity. Preserve its original timestamp and
  inclusive 90-second freshness bound. Missing, partial, stale, future or
  mismatched observations produce unknown with no additions/removal candidates.
  Invalid context can separately produce a blocked preview; source stays visible.
  A complete fresh empty inventory can propose additions.
- Match Servarr roots by exact case-sensitive path, accepting trailing slash
  normalization for comparison. Match Jellyfin's fixed ASCII name/type keys
  case-insensitively and paths exactly. Existing same-name conflicting libraries,
  duplicate normalized roots/keys and inconsistent IDs refuse an automatic draft
  rather than selecting a row or creating another same-name library. Unrelated
  roots and libraries are not changed or copied into the summary.
- Preserve the frozen unsafe-root rule as **removal candidates**, including
  `/downloads`, `/data/downloads` and declared custom download exposures. Prefix
  siblings such as `/downloads2` survive. Candidates set `review_required`, never
  become DELETE operations, and do not imply that removal is safe. Selected
  Jellyfin libraries containing download paths block the preview.
- Keep six execution gates unresolved for every outcome, including `unchanged`.
  The serializable summary cannot restore source ownership, recurring authority,
  approval, job, lock or receipt. No executable seed plan or HTTP request exists.

`executor-container::deployment::media::{root_folders,libraries}` decode supplied
response bytes only. They admit arrays of objects, bounded while decoding, and
retain root ID/path or Jellyfin name/type/locations. The frozen PascalCase and
lowercase Jellyfin aliases are supported; duplicate consumed aliases refuse.
Null/missing/wrongly typed consumed fields, malformed paths, trailing bytes and
oversize data fail with fixed errors. Environment/API-key/LibraryOptions sentinel
values are skipped without entering returned values, summaries or Debug. Unknown
metadata is not fully validated. No HTTP status, endpoint, credential scope,
completeness, sample time or trusted origin is established by decoding success.

Bounds: 32 KiB per supplied response before parsing; 16 root/library records;
8 paths per library; 8 mount declarations; 512 bytes per literal path; 128 per
library name/type; 32 KiB aggregate admitted text before summary clones; 64 KiB
streaming summary. B2 project identity bounds apply. No truncation becomes success.

## Integration and remaining parity

1. Accept alongside B2/catalog and export the new media leaves. Existing serde/
   serde_json dependencies suffice. Shared roots/manifests/lock, protocol, Store,
   schema, routes and frontend shell remain unchanged. Other B branches retain
   separate dependencies/evidence; this branch does not include network/image/
   package/prerequisite/automation changes.
2. Bind the exact selected catalog application/stack/service to protected Compose
   and runtime sources. Supply complete resolved mounts and numeric identity,
   including managed PUID/PGID where appropriate; do not trust caller DTOs as
   protected configuration. The current catalog/source subset still refuses VPN
   namespace templates; this media preview does not add that support or qualify
   an executable frozen media bundle.
3. Collect service metadata through a bounded credential-scoped read adapter.
   Enforce approved endpoint/transport/status, credential and redirect policy,
   response/time/cohort budgets, complete pagination and original source lifetime.
   A failed read or partial response is unknown, never a fresh empty list. Reuse
   B2's exact identity; do not infer a target from service name or a shared port.
   HTTP and credential adapters remain pending.
4. A supplies real mounted/physical dependencies and verified numeric ownership.
   Shared execution must protect all paths with current authority, common claims/
   locks and effect-boundary evidence. Declaration-only aliases and matching
   profile/device names cannot prove same-filesystem or container hardlink safety.
   No A-owned file was required for this independent B slice.
5. Register authenticated cache-only preview/status API and native UI centrally.
   Show source freshness, actual target, derived paths, additions, blocked reasons,
   removal candidates and unresolved readiness. No GET effect. Writes need durable
   POST/CSRF/current grants/jobs, reconnect-safe status and explicit review. Source
   data and user sessions must be cleared appropriately on expiry/grant changes.
   This branch adds no API/client/UI code or acceptance.
6. Before removal, inspect current series/movie/artist references and service
   policy; obtain separate explicit review plus backup/recovery prerequisites.
   Never replay the frozen unconditional root deletion. Before any service write,
   recollect current configuration/identity, serialize with deployment/backup,
   journal uncertain effects and verify the actual service result after restart.
   Preview differences cannot authorize changes or prove successful seeding.
7. Complete the remaining media parity rows: download-client and indexer
   application seeding, management/naming/quality settings, local Transmission/
   SABnzbd config, provisioning/layout/profile/quickstart execution and activity.
   Preserve all inventoried services; Readarr/other providers and arbitrary custom
   seed manifests are outside this initial four-application subset, visibly
   unsupported until their explicit adapter exists. Assistant media requests
   remain P07. No feature is retired by this handoff.

## Focused evidence

[Packet](rewrite-evidence/p04/2026-10-15-media-library-planning/README.md) binds
**63 passing Rust tests**: 21 media domain, 10 decoder, 20 existing catalog and
12 B2 domain. Strict Clippy, Rustfmt, Ruff 0.17.0 and harness formatting pass.
Three existing storage-profile fixtures are reused. Two synthetic response
fixtures port the frozen download-root and Jellyfin Movies cases; sentinel
credentials/options are fake. Frozen Python/catalog sources are read and hashed,
never imported or executed. The regression packet records that unresolved
download-root template syntax was accepted by the new planner before stricter
media admission; the existing shared storage schema is unchanged.

No root-positive service/Engine/physical collection, network request, service or
host write, API/UI/browser, installed recovery, whole-worker budget, package,
ARM64/Pi/performance/soak or cutover qualification ran. B2 reader/API/UI and broad
original-domain tests retain their prior separate evidence. The registered B1/B2
milestones, full media parity, RW-042 and P04 remain open. A's in-progress mount
work was inspected read-only and left intact; central registration remains a
separate dependency.
