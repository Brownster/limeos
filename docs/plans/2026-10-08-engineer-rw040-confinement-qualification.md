# Engineer 2: disposable-guest RW-040 confinement qualification

Prepare a private AMD64 local/CI guest harness to qualify production-root Engine/process/source/filesystem collection, actual confinement refusals and resource admission. Begin with guest fixtures and current library prerequisites; combined-worker acceptance waits for the integrator's supplied exact probe/worker source. No positive fixture-authentication substitute qualifies production behavior.

Start `engineer/rw040-confinement-qualification` in a separate worktree from `2fd74209e1238c1374837ab431ef670020726dc2` after [its full CI](https://github.com/Brownster/limeos/actions/runs/37746766303) succeeds. Read [the coordinator](2026-10-08-engineer-assignments.md), [Engine/PIDs](../p04-container-engine-process-evidence.md), [kernel processes](../p04-container-process-evidence.md), [host sources](../p04-container-source-evidence.md), [filesystem binding](../p04-container-filesystem-evidence.md), [integration limits](../rewrite-evidence/p04/2026-10-08-library-handoff-integration/README.md), [RW-040](2026-10-04-rust-rewrite-roadmap.md) and [defects](2026-10-04-rust-rewrite-defect-register.md).

Provisional estimate: **16 human engineering hours**, with review at **24 hours (150%)**. P04 remains 320/480. Record agent work, infrastructure/approval waits and measured human work separately. Start useful harness/fixture work while prerequisites are pending; label blocked production cases rather than inventing a weaker fallback.

## Exclusive ownership and prerequisites

- `tests/qualification/rw040-combined/`
- `tests/fixtures/rw040-combined/`
- `docs/rewrite-evidence/p04/rw040-combined-qualification/`

The integrator reserves **`bins/executor/tests/combined_read_probe.rs`**, all bin production code, library interfaces, Docker/storage/custody sources, service/capability policies, package scripts, shared workflows/ARM harnesses and public commands/RPC/routes. The qualification probe must use the bin's existing dependencies and production public APIs; no new crate/dependency or production command is included. Request the exact supplied probe/binary/source before claiming public-library success; the current base does not contain it. Only the integrator updates that seam or later combined worker. Use private harness entry points without editing CI workflows; return a proposed CI invocation for integration.

Guests, virtual disks, test daemon/containers/users, keys and supervisor belong solely to the harness. Owned disposable local/CI AMD64 guest setup, test effects and teardown are authorized. **No SSH/Pi/wybie, production-host guest, workstation/production Docker, workload/package operation or frozen Python change is authorized.** Native ARM64/Pi testing remains a separate future scope. Identify owned guests independently of shared host processes and clean all owned resources even after failure/deadline.

Use owned guest console/QMP transport under this no-SSH scope. Local-guest SSH would require a separately explicit authorization; no such exception is included here.

### Supplied current-library prerequisite

The integrator now supplies the [test-only public-library probe](../p04-rw040-combined-read-probe.md) at `dbf1aac8d1ffb3293dc85e890636a114ecbe5eca`, SHA-256 `3929400eb457937923311206201e4ef8ce2b935afc685ec7e81b27e557699b9c`. Consume that reserved test file/commit unchanged and record the exact probe/library source composition and binary hash; its source commit adds only the probe over unchanged qualified libraries. The contract gives the build/run/environment/JSON framing and success requirements. Existing-library guest cases can use this prerequisite; combined-worker/admission/supervision cases remain explicitly unavailable. [Exact-source probe CI](https://github.com/Brownster/limeos/actions/runs/37813669102) is pending and remains separate from guest qualification. This does not change the original pinned harness branch or grant shared source ownership.

## Confinement and coverage

Qualify actual UID-0 host authentication and canonical `/run/docker.sock` using guest Docker credentials and real complete running/stopped/mountless consumers. Peer UID/socket binding is not independent daemon-PID identity; record activation/endpoint facts. Unrestricted guest-root library success and unchanged-service confinement success/refusal are distinct cases.

The dormant reader has `LimitNOFILE=256`, `MemoryMax=64M`, `MemorySwapMax=0`, `TasksMax=16`, `CPUQuota=50%`, `NoNewPrivileges=yes`, empty capability sets and the current syscall/address-family restrictions. Reproduce those existing settings without granting capabilities or raising ceilings to make a case pass. The maximum process-only owner holds 261 descriptors/peaks 265 before other owners; the recorded maximum-row fixture had 31 MiB payload/65.78 MiB RSS, not universal bounds or installed signoff. Measure actual combined headroom. Whole-request unavailable/limit refusal is valid evidence and never means complete production-count support.

The first combined scope is authenticated Engine/PIDs plus kernel facts and existing **host-source** physical observations. Real virtual disks, aliases, nested mounts and named volumes exercise that scope. Do not claim namespace-destination-to-UUID mapping, compare namespace-local mount IDs directly with host IDs, or infer complete overlay/FUSE/Btrfs backing. Unsupported layouts cannot become complete empty authority.

## Acceptance matrix

| Scenario | Required evidence |
|---|---|
| Canonical authenticated Engine, empty/complete running/stopped/mountless membership | Actual public API/kernel results, original issue time and full membership; no private host-auth seam |
| Same-UID, cross-UID, nondumpable tasks; missing capabilities/ptrace/proc/statx support | Specific fail-closed outcome under unchanged confinement, separately recorded unrestricted-root success |
| Real aliases/nested filesystems/named volumes and swapped UUID/device facts | Host-source coverage bound to source/config/disk facts; unsupported destination/backing scope explicit |
| Owned restart/exit/root/namespace change and declaration/lifetime expiry | Original-lifetime and identity refusal; label any injected transition accurately |
| Empty/near/over-budget membership and concurrent requests | Actual baseline/peak/closure FD and RSS/PSS/read-traffic/latency measurements; whole-request refusal without omitted consumers or cap changes |
| Cancellation, parent death, independent deadline, blocked synchronous work | Owned worker stopped/reaped, descriptors closed, guest teardown complete with no host workload change |

The integrator owns single-flight admission and killable worker implementation. Harness cases may prepare those scenarios now but cannot claim them before that supplied worker exists. Record exact source/probe/binary/package/service-config/image hashes, kernel/page size, credentials/capabilities, disk layout and cache state, actual commands/counts and raw failed attempts. Current full-source CI and separate guest qualification are distinct. Keep the historical 61.401 ms / 20 ms failed inventory row unchanged.

Run meaningful harness checks plus applicable required fmt/strict Clippy/workspace tests, contracts/repository/dependency/diff checks for any supplied test seam, with exact source attribution. Use ordinary discovery and disclose locked/offline advisory scope. Return a clean owned-path feature branch, history/worktree, reproducible bounded invocation, proof hashes, actual results and unresolved cases; push only your assigned branch. The integrator reviews, publishes main and adds shared workflow integration separately.

**No P04 gate closes.** Installed combined resource/capability qualification, namespace destinations, pool/protection/share propagation, complete claims/ceilings/approval and effect-time verification remain prerequisites to DSK-001, MNT-001/MNT-002 and RT-001. There is no mount/unmount/restore authority in these observations, and BKP-001 remains open.
