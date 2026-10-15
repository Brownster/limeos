# Catalog namespace planning handoff

Engineer B, 15 October 2026. Branch `engineer/p04-catalog-namespaces`, base
`0972b26`. Tested code: `0ead2a40c8d123e792003d919e6e090d8beb0f9e`.
This extends the [catalog planner](p04-catalog-planning-handoff.md) in the B2,
catalog, media library/activity/client lineage. Other B branches remain separate.

## Delivered behavior

The B-owned `deployment::catalog` model now represents a closed
`NetworkMode::Service { service }` and typed `Dependency` entries containing
service, condition, restart and required. `Dependency::from("name")` expresses
the old started/false/true defaults for Rust callers. The declared Compose
dependency remains separate from catalog business `requires`.

- Render `network_mode: service:<exact service>` into the managed JSON only for
  managed consumers. Preserve long-form started/healthy/completed-successfully
  dependency conditions and restart/required flags; healthy is never reduced to
  short-form ordering. Namespace-only source adds no invented `depends_on`.
- Resolve providers inside the selected stack's effective operator/managed
  declaration. A provider in another supplied stack cannot satisfy the reference.
  Inherited operator providers remain inherited; their files are not rendered.
- Include namespace edges in missing-provider, cycle and removal checks, even
  without explicit dependencies. Reject self-reference, namespace cycles and
  mixed business/startup/namespace cycles. Support at most 16 effective services
  and bounded acyclic namespace chains, preserving each immediate reference.
- Refuse consumers with own published ports, explicit networks or host mode.
  Preserve declared provider ports; never relocate or synthesize them. Existing
  shared Compose validation still applies to the base service projection.
  Host namespace providers/chains remain unsupported in this subset.
- Refuse a completed-successfully startup condition on an immediate or transitive
  namespace provider. An unrelated setup service may still have that condition.
  This conservative rule avoids drafting a consumer that requires its namespace
  provider to have exited before starting.
- Block provider removal while either operator or managed consumers reference
  it. Removing a leaf consumer retains provider ports and other consumers, with
  the selected service still requiring verified shutdown. Optional dependencies
  retain their declared flag when present; missing optional providers and their
  removal are conservatively refused rather than pruned.

This is **catalog summary version two**. It includes explicit sorted before/
desired namespace edges alongside complete operator/before/desired models.
The base `ComposeImpact` still omits namespace and startup metadata; it is not an
execution review for this larger proposal. All eight Compose gates and eight
catalog gates remain unresolved, including the new
`VerifiedNamespaceProviderAndStartup`. Plain DTOs, successful planning and
serializable summaries confer no ownership, approval, job or receipt authority.

Admission charges dependency/provider text before cloning. Existing eight-entry
service-list, 16-service, 64 KiB text/artifact and 256 KiB summary ceilings remain.
Unknown enum/struct fields are refused. These Deserialize derives are convenient
typed fixture/declaration parsing, not a bounded IPC or complete YAML adapter.

## Integration and limits

1. Take this follow-up after the preceding B lineage. Shared roots, manifests/
   production lock, API/UI, protocol, Store/schema and packaging are unchanged.
   The standalone harness exposes the dormant module for local tests only.
2. Update proposed callers/contracts from `Vec<String>` dependencies to the
   typed entries. Bind summary version two, exact complete models, artifact and
   startup flags into central review. No version-one/P03 preview approval may
   authorize this expanded draft. Registry, current grants, executable authority,
   common claims/locks, atomic managed-file application and recovery stay central.
3. Adapt protected complete source to these types; refuse unsupported fields
   instead of dropping them. A namespace string is not a trusted Engine target.
   Resolve exact Engine/project/service/container instance and replicas through
   protected sources. Verify provider liveness, actual namespace identity and
   any required healthcheck or successful-completion result at execution time.
   Declared `service_healthy` proves no healthcheck exists or passes.
4. Qualify the installed Compose implementation. The renderer emits explicit
   long-form `required` and `restart` fields; version compatibility must be checked
   centrally. Provider replacement/restart needs whole-group reconciliation:
   `restart: true` is preserved configuration, not proof that every consumer has
   joined a replacement provider's namespace. Never use it as a verified receipt.
5. The synthetic fixture ports only the frozen Transmission/Sonarr namespace and
   healthy-dependency clauses, with fake pinned images, selected provider ports
   and literal case-sensitive paths/numeric identity. It is **not the full frozen
   VPN/media bundle**: environment/env_file, capabilities and healthcheck
   declarations are still unrepresented; the frozen gateway's nine published
   ports exceed the existing eight-port base limit. No such field is silently
   accepted or retired by this slice. A supported complete gateway/source adapter
   and genuine VPN/health evidence remain required.
6. Have the existing qualification owner run registered API/UI and disposable
   Compose scenarios: inherited provider, two consumers, selected removal,
   failed stop preserving files, provider replacement and interruption, current
   namespace/health failure and shared storage/deployment serialization. A owns
   physical dependencies; this slice required no A changes and left its worktree
   intact. Durable B1 still awaits the frozen central Store/transaction seam.

No network/process/host effect, credential read, write adapter, API registration,
UI mounting or Pi action was added. No milestone, defect, RW-042 or P04 is closed.

## Local evidence

[Packet](rewrite-evidence/p04/2026-10-15-catalog-namespaces/README.md): **85 passing
Rust domain tests**, comprising 17 new namespace cases, 20 existing catalog,
21 media library, 15 activity and 12 B2 discovery cases. Strict Clippy across all
harness targets and Rustfmt pass. No cases are skipped/filtered in that final run.
The code was committed after those checks with no intervening product changes.
Decoder/API, frontend, installed/browser, native/ARM64/Pi, worker footprint,
performance and soak suites were not rerun or credited here.

The first run had 83/84 passing: the new test incorrectly expected a numeric
published port, while the existing compiler deliberately renders it as a string.
The test was corrected to the established representation. Review then reproduced
the completed-provider contradiction as a failing focused regression before the
graph rule was fixed. Both failed runs and their source snapshots are retained.

Primary references: Docker's [network mode and dependency
syntax](https://docs.docker.com/reference/compose-file/services/),
[dependency ordering](https://docs.docker.com/compose/how-tos/startup-order/) and
[container network restrictions](https://docs.docker.com/engine/network/).
Frozen source HEAD `80593b29443ea9b81eb60ed4e5c3c3ac7a236204`; selected catalog
files are hashed, never imported/executed or changed.
