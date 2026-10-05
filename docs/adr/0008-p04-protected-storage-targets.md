# ADR 0008: Qualify protected target preparation before mounting

Status: accepted for RW-040 under the approved sequential rewrite plan.

Mount/fstab execution cannot use version-1 HTTP preview approvals. Bare targets may contain data, be writable, live beneath other filesystems or hide under existing mounts. Active swap can share a selected disk through sibling partitions or mapper backings. Single-container locks do not coordinate storage dependencies yet.

Qualify a distinct root operator preparation operation first. Bind fresh complete storage evidence and descriptor identities for empty unmounted targets. Create final directories only; require existing protected parents in the host root filesystem. Keep mounted-target preparation closed until the hidden directory can be proved safe. Preserve the HTTP preview contract and read-only reader ceiling.

Commit prepared receipts and configuration/UUID/mountpoint claims in a private bounded WAL journal before effects. Recheck evidence, create through retained descriptors, fsync and verify. Death or uncertainty keeps claims and prohibits automatic replay. Explicit reconciliation uses fresh evidence: only unchanged before-state or complete verified postconditions release claims. Preserve unexpected data.

Add an optional active-swap flag while preserving canonical JSON for ordinary devices. Bind stable swap identity into the topology digest, excluding utilization. Refuse connected swap backings in readiness and guided rendering. The installed-binary check keeps root mutation/reconciliation unavailable in the shadow payload.

Exercise installed packages, swap backing/sibling exclusion, unsafe paths, stale/old preview rejection, replay and real executor death before/after mkdir in guarded disposable Debian disks. Mount/fstab, core authorization and cross-executor locks remain the next slice. Keep their limits explicit; do not close P04 defects or infer Pi performance from target preparation.
