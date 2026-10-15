# Catalog service namespace planning

Stream B follow-up to `0972b26`. The frozen media templates declare
`network_mode: service:vpn` and a `service_healthy` dependency. The current catalog
leaf cannot represent the namespace or the long dependency condition. Simply
discarding either would weaken the proposed configuration.

Extend only the B-owned catalog model/compiler. Represent service namespace
sharing with a closed typed mode referring to an exact service in the selected
project, and represent startup dependencies with their condition, restart and
required settings. Keep catalog `requires` distinct from Compose dependencies.
Namespace edges participate in missing-reference, cycle and provider-removal
checks even when the template has no explicit `depends_on`. Preserve explicit
conditions when rendering the managed JSON; never replace `service_healthy` with
short-form startup ordering. Do not invent an explicit dependency where the
source only declared namespace sharing.

Reject namespace consumers with published ports, explicit networks or host
network mode. Refuse a namespace chain ending in host networking in this subset.
Support bounded acyclic service chains; refuse runtime `container:` references,
cross-project guesses, self-reference and mixed dependency cycles. Missing
optional dependencies are conservatively refused, rather than silently pruned.
Ports remain on their declared provider; never relocate or invent them.

Version the catalog summary to two. Include explicit before/desired namespace
bindings alongside the complete before/desired model. The existing base Compose
impact still omits catalog-specific metadata and cannot authorize this expanded
proposal. Add an unresolved namespace/provider/startup verification gate. The
complete model, exact artifact, dependency conditions and gate must be bound by
central review before execution. No protected evidence, approval shortcut,
receipt, process, network request or effect is introduced.

A synthetic fixture ports the frozen Transmission/Sonarr namespace and healthy
dependency declarations, using pinned fake images and literal paths. Tests cover
inheritance, exact targeting, provider removal, conflicts, mixed cycles, bounded
chains and deterministic rendering. Existing catalog/media/activity/B2 domain
regressions run in one isolated local Cargo harness, followed by strict Clippy,
Rustfmt and harness checks. Shared crate roots/manifests/lock, Store/schema,
protocol, API/UI mounting and A-owned work remain untouched.

This does not qualify the complete frozen media bundle. Environment/env_file,
capabilities, healthcheck declarations and other unsupported catalog fields
still require explicit adapters and protected source binding. Healthy condition
metadata does not prove a real healthcheck exists or passes. Live namespace/VPN
identity, replicas, restart recovery, lock/claim integration and installed
Compose conformance remain acceptance work for the integrator/test owner.

Primary references: Docker's [service network mode and dependency
syntax](https://docs.docker.com/reference/compose-file/services/), and
[startup/shutdown ordering](https://docs.docker.com/compose/how-tos/startup-order/).
