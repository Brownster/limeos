# Conservative typed Compose override compiler

Base: local main `746eaec`. The retained management binding checks declared files
and Engine membership, while P03 previews contain typed project snapshots. Add a
pure compiler between those boundaries; it accepts no pathname or host adapter.

Normalize bounded borrowed operator, current and desired P03 projects. Check
collection/string ceilings before cloning. Require both current and desired to
be expressible as an ordinary replacement `limeos.override.json` merged with the
same operator project. These are supplied declarations, not freshly parsed YAML
or retained evidence. Never serialize or modify the operator document.

Use the [documented Compose merge rules](https://docs.docker.com/reference/compose-file/merge/):
services merge as maps, ports have the IP/target/published/protocol key, volumes
have the target key, and ordinary sequences append. An omitted managed service
can disappear when replacing the managed file; an operator service cannot.
Operator ports and devices must remain exactly present. Operator mount targets
must remain with the same kind; supported source/read-only changes replace that
target. Refuse removal, device remapping, volume/bind kind changes, clearing an
inherited numeric user and clearing inherited host networking. No YAML reset or
override tags, null-as-delete assumption or version-sensitive clearing shortcut.

Render only added/changed services. Explicit scalar image, numeric `uid:gid`,
restart and privileged values preserve P03 contracts. Emit only new ports and
devices so append semantics cannot duplicate inherited rows. Mount deltas use
long form with explicit read-only values; new/changed binds set
`create_host_path: false`, preventing implicit host directory creation. Devices
use explicit `source:target:rwm`. Host networking is emitted only when selected.
Declare only required named volumes using empty standard definitions; arbitrary
drivers, external names, networks, environment, commands and secrets stay outside
the supported operator model. Maximum sixteen services/eight entries per service,
128 named volume declarations, and 64 KiB per compact artifact. A capped writer
refuses before extending beyond the byte ceiling.

Return a privately constructed non-deserializable plan and borrowed serialize-only
summary: normalized operator/current/desired, complete P03 impact, removed managed
service names requiring shutdown, fixed filename and artifact length. Expose
immutable bytes separately. Always retain unresolved source admission, retained
file and Engine binding, physical consumers, principal/fresh approval, stack/
storage locks, atomic writes/recovery and shutdown/effect-time verification.
The result is a reviewable proposal with no receipt, claim, approval or effect.
Empty desired projects are shutdown proposals, not permission to delete directories.
Count the complete borrowed compact summary against 256 KiB without a report
buffer; this is an admission ceiling, not peak allocator/RSS qualification.

Local tests use byte-exact synthetic artifact fixtures and a test-side merge
oracle over emitted JSON, alongside the existing P03 catalog. Cover scalar and
numeric ownership, ports/volumes/device merge regressions, elevation, supported
single managed-service removal with unrelated rows preserved, operator removals
and unrepresentable clearing refusal, invalid current input, deterministic order,
pre-clone limits, capped output and report/owner import refusal. Check the frozen
Python mutation/YAML boundary for context without importing its writer behavior.

Commit design, source/tests and evidence separately. Retain full local diagnostic
and exact-source test/Clippy/fmt/contracts/repository logs and source/artifact
hashes. Use the established on-disk build/cache paths and precompiled small cleaned
tmpfs storage fixtures only. No Pi, SSH, Docker invocation, block probe or guest.
Dependencies, generated/wire contracts, Core/Store, locks/recovery, services,
packages, external qualification and the reserved backup worktree stay unchanged.
Actual YAML/interpolation/includes, pinned Compose conformance, executable approval,
installed resource/performance qualification and cutover remain later gates.
