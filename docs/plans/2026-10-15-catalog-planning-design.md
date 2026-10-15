# B4 catalog installation proposals

Engineer B, 15 October 2026. Build on B2 `5e275ca`; no A or central files change.

Current A admission/effect/integration worktrees contain active uncommitted work.
Do not duplicate them. B3 execution awaits central executable authority, claims,
recovery and physical evidence. B4 catalog planning is independent of those effects.

Deliver pure, bounded catalog install/removal proposals. The request always names
an `(app, stack)` pair and an explicit existing/new target. Never infer a stack
from the first matching app. New-stack planning checks supplied declaration names
but does not establish actual directory absence. No parallel Store, registry,
lock, approval, receipt or execution model is introduced.

Represent supported catalog services with the existing pinned-image/numeric-user/
literal-mount ComposeService, plus explicit catalog requirements, Compose service
dependencies, named networks and config/secret grants. Support typed top-level
networks, volumes, configs and secrets. File-backed and external data resources
contain references, never secret contents; named/external volumes and bridge/
external networks form the initial conservative subset. Unknown source fields,
inline secret/config contents, environment sources, driver options, VPN namespace
semantics and unsupported grant ownership/modes require later explicit adapters.
No unknown field is silently discarded to make a template deployable.

Model operator and managed declarations separately. This slice permits distinct
managed catalog services; it refuses operator service replacement/removal through
ordinary JSON. Compile base managed services with the existing override compiler,
then add the supported typed attachments and top-level resources. The result is
one complete replacement `limeos.override.json`, never operator YAML. Actual
YAML/comments/anchors/source hashes still require the protected source admission
and atomic writer; a declaration-only proposal does not verify their preservation.

Resource merge follows the frozen rule: equal definitions reuse existing entries;
different definitions conflict in every section. Existing managed definitions
survive replacement, including unreferenced resources. Removing an app removes
only its managed service, retains other services/resources and requires verified
shutdown of that one service before replacement. Dependent services block removal.
No request option bypasses dependencies, stops the whole stack or deletes a stack
directory. Orphan policy remains an explicit reviewed field.

Keep catalog `requires` distinct from Compose `depends_on`; the former never
silently changes startup ordering. Validate references against the selected
effective project. Preserve numeric UID/GID and case-sensitive paths. Do not
template substitute or load any referenced file. Packages/source adapters must
supply fully resolved, complete supported declarations and current metadata.

Bounds: 8 supplied stacks; 16 effective services in the selected stack; 8 entries
per service dependency/network/config/secret list; 16 resources per section in
each source; 48-byte aliases; 512-byte resource paths; 128-byte external names;
64 KiB selected aggregate text and artifact; 256 KiB serialized review summary.
Admission precedes clones. Sort output and changes deterministically. Plans keep
all existing unresolved execution gates and expose no effect or receipt API.

Focused tests cover the frozen resource regression, collisions in all four
sections, shared definitions, same app in two stacks, dependency/refusal cases,
single-service removal, retained resources, numeric ownership, literal paths,
unresolved placeholders, unsupported fields, determinism and limits. Broad VM,
installed Compose, ARM64/Pi and recovery qualification remain with the test owner.

Reference: frozen pi-health `80593b29443ea9b81eb60ed4e5c3c3ac7a236204`,
`catalog_service.py` and `tests/test_catalog.py` (resource regression around line
1000). Docker primary references: [configs](https://docs.docker.com/reference/compose-file/configs/),
[secrets](https://docs.docker.com/reference/compose-file/secrets/),
[merge rules](https://docs.docker.com/reference/compose-file/merge/).
The planner's subset/refusals are narrower than the full Compose specification.
