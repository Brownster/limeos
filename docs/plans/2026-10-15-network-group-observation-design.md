# Read-only shared-network groups

Engineer B, 15 October 2026. Base B2 `5e275ca`. Central runtime registration and
A's mount work remain separate. This slice covers the read side of RW-042's
preserved network grouping; recreation and public-IP probing remain pending.

Use a pure topology function and a selective, bounded Engine-inspect byte adapter.
Extending the protected collector now would cross central ownership. Reusing
global names or the first dependency label as authority would repeat a frozen
failure mode. Instead retain declarations as diagnostics and require exact
instance/project identity for provider links. No Store, protocol, lock, owner,
approval, job, effect or receipt type is introduced.

The pure input is ordinary display data: expected/observed Engine identity,
original cohort time, explicit complete/partial collection, and at most 16 nodes.
Unavailable, incomplete, stale, future or mismatched cohorts produce unknown
counts, never a successful empty inventory. A complete fresh empty cohort is
distinguished. Even a fresh report describes configured references, not actual
process namespace equality, VPN protection or network reachability.

Resolve `container:` by full lowercase ID or exact case-sensitive container name.
Short ID references are diagnosed as unsupported rather than expanded. Conflicting
ID/name matches stay ambiguous. Resolve `service:` only within an identical B2
Compose project/directory/ordered-file identity, to one ordinary replica; one-offs
cannot satisfy that reference. No global service-name fallback or running-replica
preference. Self-reference, cycles and chains stay visible and cannot be healthy.

Preserve stopped members, provider running/health state and orphan diagnostics.
A single well-formed frozen dependency label may supply a project-scoped display
hint for a dead provider ID. It never repairs the reference or changes an orphan
into a resolved member. Multiple dependencies have no preferred hint. Groups
with hinted orphans are degraded. Unconfigured health differs from missing health;
no configured health-check object means only that the provider was observed
running. A present check object without an observed result remains unknown,
including disabled checks: command/test contents are deliberately not interpreted.

The adapter parses one inspect response against an expected full container ID.
It consumes name, running/paused/restarting/dead flags, network mode, health status
and a single dependency hint. Only normalized availability and selected display
metadata survive. Compose membership is supplied separately by the existing B2
projection; this adapter does not reconstruct source ownership from labels.
Ignored config/environment, health logs and other payloads never enter reports,
Debug or error strings. Duplicate consumed JSON fields, malformed consumed data,
identity mismatch and oversized input fail closed. Parsing does not fetch bytes,
launch an inspection or establish completeness/freshness/trusted origin.

Bounds before projection: 32 KiB per inspect response; 128 bytes per container
name/mode target, 256 per network-mode text, 512 per dependency label; 16 members,
24 KiB aggregate admitted text and 64 KiB serialized report. Apply existing B2
project path/file/name bounds. The collector must separately bound aggregate raw
bytes and whole-worker time, memory, descriptors and response size.

Focused fixtures map frozen `network_group_service.py`, `container_helpers.py`,
`tests/test_network_group_service.py` and `tests/test_containers.py` at pi-health
`80593b29443ea9b81eb60ed4e5c3c3ac7a236204`. Read/hash that source only. Tests cover
provider/member/orphan/health cases, exact identity and cross-project replicas,
one-offs, short IDs, cycles/chains, privacy, malformed data, determinism and bounds.
No broad hardware/package/root-positive qualification runs in this slice.

Docker documents the [container name/ID network reference](https://docs.docker.com/engine/network/)
and Compose's [service/container network modes](https://docs.docker.com/reference/compose-file/services/#network_mode).
This conservative observation subset is narrower than Docker's complete syntax.
