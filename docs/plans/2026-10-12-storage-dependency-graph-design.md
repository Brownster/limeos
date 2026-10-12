# Declared storage consumer graph

Start from pushed main `af94f19ac21f9da14b207439808ffb33c87e8e10` on
`engineer/storage-dependency-graph`, disk-backed `.worktrees/storage-dependency-graph`.
The integrated Pi candidate remains frozen at `016ba49`; this new branch cannot
enter it silently. Backup-bound recovery stays with its current owner.

Estimate eight human engineering hours, review at 12. P04's 320-hour estimate
and 480-hour review are unchanged; actual human/agent effort remains unmeasured.

The merged configured-dependency and Samba planners now provide complementary
inputs to RW-040/RW-041 unmount/runtime-loss planning. Deliver their composition
as one pure serialize-only graph. A caller-side join would duplicate matching
and bounds; editing either existing preview would change its frozen format.
Prefer an additive domain module and independent output version instead.

Reuse both public planners. Require validated contract/pool/protection/share
configuration and fresh bounded Docker declarations. Return ordered devices with
their existing full configured pool/protection/container row plus share consumers.
Each share consumer references its exact unique name in one retained desired
share configuration and carries its direct/pool route and ordered branches.
This avoids duplicating comments/user lists for every expanded device match.
Join requirements by complete expected device metadata, never by UUID/ID alone.

Preserve original Docker observation time/engine identity, full device identity,
numeric media UID/GID, meaningful branch/generated-path/share order, all flags,
stopped containers, disabled pools/protection/shares and read-only consumers.
Sort share consumers deterministically by name/route and retain the Samba
planner's explicit unmapped-share list. Empty shares remain an explicit caller
input and still validate their surrounding configuration.

Enforce one 512-match ceiling across all expanded container and share routes,
including ancestor/pool amplification. Individually valid previews whose sum
exceeds this limit refuse the whole composition; no partial graph or clipping.
Static pool/protection records retain their existing separate configuration
bounds. Errors use only fixed fields/codes. No clocks, filesystem/process access,
new dependencies, generated wire types, schema, registered operation or service
limits are added. Existing preview formats/functions stay unchanged.

Test four existing layout goldens, exact full-row reuse, branch/role/ownership
preservation, disabled/stopped/read-only consumers, ancestor expansion, unmapped
and literal case/component relationships, input reordering, original timestamp,
invalid/stale inputs, input immutability and mixed 511/512/513-match boundaries.
Run focused/full Rust checks, strict Clippy, fmt, contracts and repository checks
with all builds/logs/general scratch on disk. Existing kernel storage fixtures
alone use small auto-cleaned tmpfs scratch.

This remains a configured view: no actual Samba/global/account state, remote
share/provider completeness, directory existence, namespace/UUID/backing
composition or retained descriptor lifetime is established. Unmapped or empty
rows never authorize unmount. Protected collection, complete claims, approvals,
storage locks, effect-time validation, runtime-loss shutdown and durable recovery
stay integration work. No P04/defect/cutover gate closes.
