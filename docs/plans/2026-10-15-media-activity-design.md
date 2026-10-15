# Read-only media activity

Engineer B, 15 October 2026. Base media-library branch `1f64979`. Shared runtime
registration is still pending; A's dirty mount work does not prevent this B slice.
Keep A files intact for the returning agent. No second storage/Store/authority
model is introduced.

Port the frozen activity response transformations for Jellyfin/Emby sessions,
SABnzbd queues, Transmission torrents and Sonarr/Radarr/Lidarr/Readarr queues.
Use closed explicit providers and full container instance identity. Name/image
substring matching, arbitrary published-port fallback and privileged credential
file discovery are not suitable evidence of the intended endpoint. They remain
collector integration concerns rather than being copied into these decoders.

Deliver bounded supplied-byte decoders, pure scoped snapshot aggregation, an
authenticated cache-only API leaf and a presentational React activity panel.
Actual HTTP/credential/Engine collection, cache ownership, shared route/schema
and frontend shell registration remain central handoffs. The panel accepts a
validated snapshot and refresh callback; it has no background poll or mutation.
Use the existing compact, utilitarian dashboard styling, accessible tables and
plain unknown/partial states. No shared CSS or package dependency changes.

Carry original collection and per-service sample times; expire activity after
eight seconds, matching the frozen cache interval. Explicitly distinguish stale,
future, partial, unavailable, credential, timeout and invalid-response states.
A missing/failed service must not look idle or hide healthy peers. Aggregate
totals become unknown on incomplete observations; queue problem totals remain
unknown when only one page is inspected. Show observed page counts separately.
Only a complete fresh empty provider inventory can report zero.

Preserve tick-to-seconds/progress, pause/transcode information, download rates and
sizes, Transmission download/seeding status and queue problem rules. Bound and
validate numeric values, normalize percentages, and retain unknown rather than
inventing zero for absent rates/counts. Avoid attributing SABnzbd's overall rate
to multiple downloads. Transmission RPC success must be explicit; its 409 session
handshake remains a transport responsibility, not a write operation here.

Before parsing: 32 KiB per body, 24 rows per provider, eight transcode reasons.
Aggregate: eight targets, 32 KiB selected text, 12 displayed streams/downloads,
three displayed queue items per provider, 64 KiB streaming response. Expose
omitted display counts; do not silently truncate acquisition. Unknown metadata,
API tokens, URLs, file paths, session tokens and raw exception text stay out of
the projected result. Deliberately selected titles/users/devices remain private
authenticated UI data and must not be logged as diagnostic errors.

Fixture ports use frozen pi-health `80593b29443ea9b81eb60ed4e5c3c3ac7a236204`
activity_service.py and tests/test_activity_service.py, read/hashed only. Focused
checks cover all providers, missing/paginated/bad data, restart identity, freshness,
limits, privacy, cache-only authentication and static UI states. They do not
qualify live service endpoints, credentials, namespace identity, whole-worker
resources, browser interaction, installed packages, Pi performance or cutover.
