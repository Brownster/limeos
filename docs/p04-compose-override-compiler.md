# Typed managed Compose override proposals

`limeos_domain::compose_override::plan_managed_override(operator, current, desired)`
compiles a bounded replacement `limeos.override.json` from three borrowed P03
projects. It performs pure logic only. The operator project describes the supported
base file; current and desired describe effective projects before and after the
proposed replacement. Callers still need a supported source adapter to establish
these declarations from actual protected files. No YAML body is parsed or rewritten.

Both effective projects must be expressible against the same operator model.
The compiler follows the supported ordinary [Compose merge rules](https://docs.docker.com/reference/compose-file/merge/):
services merge by name, ports by IP/target/published/protocol, mounts by target,
and ordinary device sequences append. It emits a replacement managed file, which
is combined with the operator file rather than with the old managed file.

An operator service, port or device must remain present. An operator mount target
must remain with the same bind/volume kind; its source and read-only value can
change. Clearing inherited numeric ownership or host networking, removing mount
targets, remapping inherited devices and switching mount kinds refuses with
`Conflict`. Empty lists and omitted keys never stand in for deletion tags. The
compiler uses no YAML `!reset`/`!override`, version-sensitive clearing convention
or null-as-delete shortcut.

Managed additions can be removed by omission from the replacement. Removing one
managed service keeps unrelated operator and managed services. The full P03 impact
records before/after settings, additions/removals and desired elevation. Removed
service names appear in `shutdown_required`; this is a requirement for later
verified shutdown, not a shutdown receipt. An empty desired project can describe
the last managed service's removal and grants no directory or volume deletion.

Changed or new services emit immutable image digests, explicit restart/privileged
values and numeric `uid:gid` when specified. Long-form ports retain IP, published
port, target and protocol. Only newly added ports/devices are emitted, avoiding
duplicate inherited sequences. Changed/new mounts retain their literal sources
and targets and explicit read-only value. Bind mounts include
`create_host_path: false`; device mappings use explicit `rwm`. Host mode is emitted
when selected. Referenced named volumes receive deduplicated empty declarations.
Custom volume drivers/external names, arbitrary networks, environment, commands,
secrets and other unsupported operator fields require a later model/adapter.

Collection and string ceilings are checked before project clones. Existing P03
validation and deterministic sorting then apply to all three projects. Bounds are
sixteen services, eight ports/mounts/devices per service, up to 128 named volume
declarations and 64 KiB per compact artifact. Validate the declared current
combination too; it does not become a retained before-image or recovery input.
Count the complete borrowed summary against 256 KiB without a report buffer.
These are admission ceilings, not peak allocator/RSS or installed qualification.

`ManagedOverridePlan` owns immutable artifact bytes and normalized proposal facts.
`artifact()` borrows those bytes; `summary()` returns a borrowed serialize-only
report containing the complete impact and unresolved gates. Neither type can
deserialize. The summary omits artifact bodies and has no approval, receipt or
claim. Its fixed gates cover supported source admission, retained file binding,
Engine runtime binding, physical consumers, principal/fresh approval, stack and
storage locks, atomic writes/recovery, and shutdown/effect-time verification.

[Local acceptance](rewrite-evidence/p04/2026-10-14-compose-override/README.md)
uses two byte-exact synthetic fixtures, an independent test-side JSON merge
oracle and the existing P03 catalog. It qualifies the typed subset, rather than
actual YAML/.env/includes/interpolation or a pinned installed Compose binary.
See the [design](plans/2026-10-14-compose-override-compiler-design.md).

The [strict JSON source adapter](p04-compose-json-source.md) now checks actual
supported source bytes, combines their declarations and re-admits generated
artifacts. Shared local fixtures pass an independently pinned configuration parser;
general YAML and other versions remain unqualified. The
[retained management binding](p04-compose-management-binding.md) stays separate.
This compiler adds no CLI/RPC/route, Core/Store job, lock, write, deployment, service
or package activation. Retained source integration and a fresh approved operation
using locks/recovery remain next. P04, installed performance and cutover gates
remain open.
