# Declared pool and protection dependencies

Base `1401c255d4a05b8621c2dcd497d6f7f0cbe91d56`; branch
`engineer/pool-protection-dependencies`; worktree
`/home/marc/Documents/github/lime-os/.worktrees/pool-protection-dependencies`.
The worktree and build cache moved off tmpfs at the operator's request; staged
changes and every copied file were verified before deleting the original copies.
Future builds, logs and general temporary files use disk-backed `.cache/`
directories. Existing kernel storage tests use a small auto-cleaned tmpfs scratch
directory because their fixtures require matching stat/mount-table devices and
directory link-count semantics; the workstation's Btrfs scratch did not qualify.
RW-040/RW-041 need pool and protection
consumer propagation before unmount planning. Backup recovery/durable preparation
remain integrator-owned; the Pi qualification freeze remains independent.

Estimate 12 human engineering hours, slice review at 18. P04 remains 320 hours
with a separate 480-hour cutover-scope review. Actual human and agent work are
unmeasured; dates/tool waits do not establish the review threshold.

Deliver an additive pure `limeos_domain::configured_dependencies` module. Reuse
the validated storage contract, pool/SnapRAID planners and fresh bounded Docker
declarations. For every configured device, report its pool branches, generated
protection path requirements and container source/destination matches. Preserve
numeric media ownership, literal case, source volume name/driver, writable flag,
full container identity/running state and meaningful branch order. Keep disabled
pools/protection and stopped consumers visible. An ancestor source overlaps both
direct devices and configured pool roots; a pool match propagates conservatively
to every member, including branches backed by the same configured device.

Sort devices, pool/drive records and container matches deterministically without
reordering branch or generated protection requirements. Refuse the entire result
for malformed/ambiguous configuration, unknown UUID/path/role relationships,
stale/future/invalid Docker declarations or more than 512 expanded container
matches. Do not truncate. Findings use fixed field names/codes without input
values. Input bounds come from the existing validators; no new dependencies,
clock acquisition, filesystem access, process execution or persisted state.

Physical namespace-to-UUID mapping and retained pool/protection/share propagation
would require new protected collectors and installed qualification. Effectful
unmount/runtime-loss handling additionally requires complete claims, approvals,
storage locks and recovery. This declared preview is a useful independent input
to those integrations; it establishes none of their authority. Named-volume
aliases, symlinks, nested mounts and backing graphs cannot be resolved lexically.
Shares remain an explicit missing input, never inferred as absent.

Own only the new module/tests, one additive export, dedicated golden fixtures and
this slice's docs/evidence. Keep existing planners, core/schema/IPC/HTTP/CLI/UI,
service caps and other worktrees untouched. Test the four frozen layouts, pool
propagation, disabled/stopped consumers, direct paths, ancestor/case/component
boundaries, volume identity, protection roles, deterministic ordering, invalid
inputs and 511/512/over-limit expansion. Run fmt, focused/full Rust tests, strict
Clippy, contract drift and repository checks. Return a clean committed branch
with source-bound logs/hashes. DSK-001, MNT-001/MNT-002, RT-001, SRA-001/SRA-002,
BKP-001, P04 and cutover stay open; inventory 61.401 ms/20 ms stays failed.
