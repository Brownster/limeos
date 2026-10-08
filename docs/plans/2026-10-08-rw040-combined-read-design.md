# RW-040 probe first, bounded collector next

Base: clean main `02870d5`, with qualified runtime/fixture source `2fd74209`. Estimate: 16 human engineering hours; review at 24 hours. P04 remains 320/480. Agent time and infrastructure waits are recorded separately.

The first deliverable is only `bins/executor/tests/combined_read_probe.rs` plus its interface/design evidence. It uses existing bin dependencies and production public APIs. Ordinary Cargo tests exercise a contract/default case with no host I/O. Explicit guest invocation selects real root-authenticated cases; there is no fixture provider or UID override.

## Alternatives and choice

1. **Deliver the test-only probe now**, then add a reusable unregistered bin-library collector/supervisor. This unblocks guest qualification before composition is complete and makes missing guarantees explicit. Recommended.
2. Add a public executor command or RPC. This would expand the runtime surface and is outside scope.
3. Put composition/supervision only inside the qualification harness. That would couple proof to a private substitute and leave no reusable implementation to qualify.

## Probe contract version 1

Build with `cargo test -p limeos-executor --test combined_read_probe --locked --no-run --message-format=json`. Select the integration-test executable from Cargo's `compiler-artifact` record. Run it with `--exact qualification_probe --nocapture --test-threads=1`.

The trusted guest harness sets `LIMEOS_RW040_PROBE_CASE`. Supported cases:

- `contract` (default): emit available cases/prerequisites; no host reads.
- `engine`: canonical authenticated Engine collection and fresh revalidation.
- `engine-processes`: genuine Engine declarations/PIDs, public retained process collection and revalidation, bracketed by the retained Engine owner's fresh GET validation.
- `engine-sources`: genuine Engine declarations and public host-source retained collection/revalidation, bracketed by Engine validation.
- `storage`: public retained block/mount/fstab inventory collection/revalidation.
- `engine-dependencies`: trusted protected storage contract plus genuine Engine declarations and public retained host-source/filesystem dependency collection/revalidation, bracketed by Engine validation.
- `combined`: explicit `blocked` prerequisite until the bounded worker implementation exists; never a positive substitute.

`LIMEOS_RW040_PROBE_SOCKET` chooses only a trusted in-process test endpoint, default `/run/docker.sock`; the production `EnginePeerPolicy::root()` remains fixed. Paths must be absolute and protected socket collection still rejects symlink ancestors. `LIMEOS_RW040_PROBE_CONTRACT` is required only for dependencies and is read through production `read_contract`. No caller-selected Engine peer UID, proc root, declarations/PIDs, clock or private authentication seam is accepted.

Case names are at most 32 UTF-8 bytes, socket/contract paths at most 512 UTF-8 bytes, and the pause input at most four decimal bytes. `std::env::var_os` first copies the OS values; Config checks these bounds before parsing or making further path copies. Each event writer fallibly reserves a fixed 256 KiB buffer, then serializes borrowed snapshots directly within that cap; no unbounded `to_value` or `to_vec` is used. Build the complete event before writing its prefix/body so a limit failure emits no partial event. Serialization/write/flush failures are fatal probe execution failures and never a collection refusal or successful execution.

Each event line starts with `LIMEOS_RW040_PROBE_JSON=` followed by a JSON object with `version`, `case`, `phase` (`collected` or `final`), `status` (`ok`, `refused`, `invalid_input`, `blocked`), `stage`, `error` (stable tagged library error where present), and `observations`. Observations are serialized reports, never imported authority. The probe records its effective UID and original declaration issue time; Engine-backed results include complete membership/running bindings and the chosen public owner snapshots. Qualification requires successful probe exit plus exactly one `final` event with the expected case/status. A `collected` event means only that owners were acquired, not that revalidation passed. Flush failure can occur after valid JSON bytes were written, so JSON alone cannot qualify success. No error maps silently to an empty successful inventory.

An optional `LIMEOS_RW040_PROBE_PAUSE_MS` (0..5000, default zero) pauses after the flushed `collected` event while owners remain retained. It enables the guest to measure descriptors/RSS and trigger real restarts/exits/namespace changes before final revalidation. It changes no clock or authentication provider and consumes the original lifetime; expiry remains a refusal. External supervision still owns probe termination. Input validation runs before any selected host collection.

After the pause, every Engine-backed owner case performs fresh retained-Engine validation, then the selected retained process/source/dependency owner's revalidation, then fresh retained-Engine validation again. The Engine-only case performs its fresh validation. The storage-only case revalidates its retained storage owner. Final events are emitted only after that sequence; no owner is recollected or assigned a later declaration issue time.

Final `ok` records completed owner revalidation. Blocking stdout delivery can consume more time after that check: external supervision and current-age checks remain required, and the report is never effect authority. Each case emits at most one collected event and one final event, so event bodies total at most 512 KiB plus the fixed prefix/newline bytes; libtest's own progress output is separate.

Probe results qualify only the named current public libraries. External supervision is required for standalone probe deadlines; blocked synchronous reads are not bounded by an async timeout claim. Running-namespace destination mapping, independently killable composition, admission, source configuration retention, pool/protection/share propagation, claims, approvals and effects remain open.

## Next collector boundary

The reusable unregistered collector will live under the executor bin (which already depends on both adapters), not add a cross-adapter dependency. It will retain Engine, process and host-source/filesystem owners in one child and use the earliest original issue time. Collection and fresh reacquisition are sequential and compare complete consumers; no later phase mints a lifetime. A single-flight supervisor must refuse overload, enforce a wall deadline independently of child synchronous work, observe cancellation/parent death and reap all owned children.

Admission must preserve installed `LimitNOFILE=256`, `MemoryMax=64M`, `MemorySwapMax=0`, `TasksMax=16`, CPU/no-new-privilege/capability/syscall restrictions. Whole-request refusal is preferable to omission. The existing process library can retain 261 FDs before other owners, and its maximum-row fixture reached 65.78 MiB RSS: composition cannot simply concatenate maximum owners. Before implementation, the concrete resource policy must bound retained descriptors and parser payload before oversized allocation, including raw mount-table expansion. Kernel/service caps remain independent final guards; OOM/refusal cannot become a successful partial report.

The initial worker covers authenticated Engine/PIDs, retained kernel facts and existing host-source observations. Namespace-local mount IDs are not host identity and do not complete destination-to-UUID proof. Service/RPC/route/schema/claim/UI/effect changes and cap relaxation remain excluded. The worker launch/entry seam must remain test-only/unregistered until separately authorized runtime integration.

## Validation and ownership

Probe contract/invalid-input tests run locally without Docker/root/host mutation. Guest positive/refusal cases are owned by the external qualification engineer. A root/nonroot local isolated Unix-socket negative case may prove refusal without contacting workstation Docker. Targeted tests, fmt/strict Clippy, repository/contracts/dependency checks and independent exact-source review precede publication; meaningful changed-runtime checks/full exact-source CI follow the collector implementation.

Reserved external paths are untouched: backup preparation source/tests/fixtures/evidence and `tests/qualification/rw040-combined/**`, `tests/fixtures/rw040-combined/**`, `docs/rewrite-evidence/p04/rw040-combined-qualification/**`. No wybie/Pi/SSH/workstation Docker/frozen Python work. No P04/defect/cutover row closes.
