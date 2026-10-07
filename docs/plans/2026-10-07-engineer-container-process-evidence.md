# Engineer 2: retained running-process and mount-namespace evidence

Implement the next independent RW-040 kernel-evidence library. The existing Docker declarations have no host PID; retained host paths and UUID/backing probes do not inspect running-container namespaces. This slice binds supplied validated declarations and PID inputs to retained kernel observations. **It does not authenticate that a PID belongs to Docker, complete physical dependency authority or authorize an operation.**

Start `engineer/container-process-evidence` in a separate worktree from `d324c1b9f3ee8dcc46d7d5deb4cf00c25b1301f4`. Read [the coordinator](2026-10-07-engineer-assignments.md), [source descriptors](../p04-container-source-evidence.md), [filesystem binding](../p04-container-filesystem-evidence.md), [container declarations](../p04-container-dependencies.md), [RW-040 and the phase gates](2026-10-04-rust-rewrite-roadmap.md), [the defect register](2026-10-04-rust-rewrite-defect-register.md) and [current qualified source](../rewrite-evidence/p04/2026-10-07-handoff-integration/README.md). Baseline: 291 Rust tests; per-architecture CI has 33 ARM harness tests, with 9 separate reference tests and 180 installed AMD64 groups.

Provisional human estimate: **24 engineering hours**, with assignment-scope review at **36 hours (150%)**. Record agent time, infrastructure waits and measured human time separately. P04 remains 320 hours with a distinct 480-hour cutover-scope review.

## Exclusive ownership

- `crates/executor-storage/src/processes.rs`
- `crates/executor-storage/src/processes/tests.rs`
- `tests/fixtures/container-process-evidence/`
- `docs/p04-container-process-evidence.md`
- `docs/rewrite-evidence/p04/rw040-process-evidence/`

Two explicit shared-file exceptions: add only `pub mod processes;` to `crates/executor-storage/src/lib.rs`, preferably in its own small commit; if safe pidfd polling requires it, change only this crate's existing rustix declaration to `rustix = { workspace = true, features = ["event"] }` in `crates/executor-storage/Cargo.toml`. Rustix 1.1.5 is already pinned. Keep these allowances separate from substantive code so integration stays simple. No new crate, package/version, workspace dependency edit or policy/capability change is included.

Reuse existing internal helpers without editing `sources.rs`, `dependencies.rs`, `inventory.rs`, `mounts.rs`, target journals or domain contracts. Core, Docker collection/PID acquisition, persistence, IPC/API/CLI, generated contracts, operation registration, claims/receipts, package/service policy, ARM harnesses, UI and backup files belong to the integrator. Return any further shared-interface need as a concrete proposal before changing its ownership scope. The other engineer owns the archive store; the integrator continues bounded combined read-only collection outside your files.

## Interface and kernel boundary

Expose an API under `limeos_executor_storage::processes`, without a route or runtime command. The intended seam is:

```rust,ignore
pub struct RunningContainerProcessBinding {
    pub container: limeos_domain::ContainerSnapshot,
    pub pid: u32,
}
pub fn inspect_running_container_processes(
    declarations: &limeos_domain::ContainerStorageInventory,
    bindings: &[RunningContainerProcessBinding],
) -> Result<RunningContainerProcessEvidence, crate::Failure>;
impl RunningContainerProcessEvidence {
    pub fn snapshot(&self) -> &RunningContainerProcessSnapshot;
    pub fn mounts(&self, resource: &str) -> Option<&[ContainerProcessMount]>;
    pub fn revalidate(&self) -> Result<(), crate::Failure>;
}
```

Require exactly one binding for every running declaration, including consumers with no declared storage source. Reject duplicate, extra, missing or stopped bindings; require equality of the full container resource/image/running/start snapshot. Refuse PID 0/1 and values outside the supported Linux PID range. Stopped consumers remain covered by existing source evidence and are explicitly absent from this running-process object; that absence is not fresh Engine proof of their state.

Privately retain each pidfd, proc directory, mount-namespace descriptor and process root descriptor. Record host boot identity, complete declaration and binding digests, full container/PID facts, kernel start ticks, numeric UID/GID facts, namespace device/inode, root identity and complete raw mount-table digest. Keep bounded parsed namespace mount facts available through borrowed read-only access. Snapshot data is Serialize-only observation; no deserialization, public fixture provider or record import can reconstruct an evidence owner. Omit command line, environment and unrelated private records.

Bracket collection/revalidation with liveness and identity checks. A pidfd and concurrently opened proc directory must demonstrably refer to the same still-live process; reject dead pidfds, changed start identity and observed replacement. Use retained directory-relative proc reads and compare retained and freshly obtained namespace/root identities without trusting a reused numeric PID. Namespace descriptors retain identity but do not freeze mount contents: compare complete raw table digests before and after reads. Mount IDs belong to their namespace and cannot be directly equated to host mount IDs as physical filesystem proof.

Use the existing protected host context and fixed proc paths constructed only from validated PIDs. Production callers cannot select a proc root/path or inject kernel facts. Denied ptrace/proc/namespace access and unavailable pidfd/statx support fail closed. UID 0 with an empty capability set may be unable to inspect another UID's namespace/root: document actual requirements, leave service policy to the integrator and never substitute weaker metadata. Production collection/revalidation sends no signals, enters no namespace and performs no mount or data write.

`revalidate()` checks retained kernel facts and the original evidence lifetime. It cannot reacquire Engine facts, refresh their timestamp, resolve destinations, prove UUID/backing identity or authorize an effect. The integrator must acquire authenticated Engine identity/container/PID facts before and after this collection, then compose namespace/destination observations with fresh host source and filesystem evidence. Unsupported evidence refuses the entire result; it cannot become an empty success that claims no consumers.

## Bounds and acceptance matrix

Keep the existing 64-container limit and five-second declaration age, with wall-clock validation and a conservative remaining monotonic lifetime that cannot renew on revalidation. Check freshness before/after collection and revalidation. Use a two-second cooperative work budget; synchronous proc/filesystem operations still require the integrator's later independent process deadline and single-flight admission.

Use closed limits before reading/allocating: process records at most 16 KiB each, complete mount tables at most 1 MiB each/8 MiB aggregate, at most 4,096 parsed rows per table, bounded descriptors proportional to 64 bindings and a 48 KiB metadata report. Reject amplification rather than clipping or omitting consumers. Record peak allocation/descriptor counts at the admitted maximum; these are initial library bounds, not production inventory signoff. Propose a tighter compatible bound if necessary.

| Scenario | Required result |
|---|---|
| Complete running membership, including no-mount consumers | Deterministic complete observations and borrowed mount facts |
| Wrong full snapshot, missing/extra/duplicate/stopped binding, zero/system/out-of-range PID or count overflow | Whole collection refuses |
| Spaces/parentheses/control bytes in proc stat comm; malformed/truncated/oversized stat/status/mount data | Safe parsing or explicit refusal; comm/private content never reaches reports |
| Real owned local child, then exit during collection/revalidation | Live identity retains correctly; exit invalidates; Drop closes all handles |
| Injected PID reuse between pidfd/proc opens | Replacement cannot satisfy old evidence |
| UID/GID, root or namespace changes with otherwise matching PID/start | Refusal without stale-identity fallback |
| Complete table changes, including parent/propagation/options-only changes; same-looking rows in different namespaces | Raw digest/namespace mismatch refuses |
| Detached/unsupported/replaced descriptors, denied access or missing kernel primitives | Explicit whole-result failure; no capabilities added |
| Future/expired declarations, age crossed while reading, wall-clock rollback plus monotonic expiry or repeated revalidation | Original lifetime remains enforced |
| Work/byte/row/report/descriptor exhaustion | Bounded refusal, never partial success |
| Serialized or caller-injected snapshot | Cannot construct retained evidence or operation authority |

Tests use private local scratch fixtures and short-lived owned child processes. They require no Docker, root guest, setns/unshare, mount, block-device or production operation. Inject difficult kernel interleavings through private test seams; clearly label synthetic namespace-change proof and preserve genuinely untested installed scenarios. No SSH, wybie/Pi access, expired-window reuse or frozen Python changes are authorized.

## Required checks and handoff

Run targeted meaningful Rust tests, then `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `cargo test --workspace --locked`, `cargo deny check`, `cargo audit`, `python3 scripts/check_repository.py`, `python3 scripts/check_contracts.py` and `git diff --check`. Use locked/offline caches where needed and label cached advisory scope/environment failures accurately. These Python scripts validate the Rust repository; the original Python project stays untouched. Unchanged frontend/native/Pi workloads do not need a new local run for this library.

Return a clean branch with small commits, exact base/tip/implementation/test identities, allowed-path diff, interface document, actual test counts, raw passing checks and retained failed regression attempts. Bind source/fixtures/output hashes; distinguish real child-process proof from injected proc/clock/namespace data, and record resource bounds and remaining capability/service decisions. Give the commit list, branch and absolute worktree path. You may push only your assigned feature branch to `Brownster/limeos`; do not merge/push main or change historical proof. The integrator owns main publication, review and full exact-source merged CI/installed qualification.

**No gate closes.** Engine ownership/reacquisition, namespace-to-source/UUID/backing composition, pool/protection/share propagation, approvals, complete shared claims, root ceilings, bounded service admission, installed collection, effect-time checks, mount/fstab execution, unmount and runtime-loss shutdown remain integration work. DSK-001, MNT-001/MNT-002, RT-001 and applicable P04 rows remain open; the 61.401 ms/20 ms inventory failure is unchanged.
