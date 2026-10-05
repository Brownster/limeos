# ADR 0007: Fresh evidence and durable guided storage previews

Status: accepted for the next RW-040 slice. P04 remains in progress.

The qualified readiness command checks already mounted assignments. Guided setup also needs to select unmounted existing filesystems and review fstab ownership. Dashboard observations and cached `/dev` names cannot supply that authority, and the generic host service uses a private device/mount view. Fstab options may contain remote credentials.

Add a dormant, dedicated root inventory reader with its own read-only ceiling. Keep the established host service unchanged. The reader re-probes retained descriptors, compares the connected boot topology and full mount table, reads protected fstab metadata, and bounds admission/output/time. Root kernel credentials authenticate the reader to the core. Sanitized identity/target fields and digests leave the reader; fstab options stay private. Unresolved local aliases fail closed.

Persist a closed guided profile contract and deterministic managed fstab section against that complete inventory. The browser selects a digest; creation and human approval both obtain fresh evidence. The database binds principal, grant revision, expiry and plan digest, and hashes approval tokens. Review/cancellation can proceed without hardware collection. No executable intent or queue exists in this version; future mount execution needs a new version and new approval, per-resource locks, protected target preparation, a prepared receipt before effects and verified/reconciled outcomes after interruption.

Support ext2/3/4 and XFS in setup previews first. Leave Btrfs multi-device, FUSE/removable ownership, remote aliases and vendor serial proof open. Preserve all frozen contract profiles and filesystem enums so the remaining parity work is explicit. Do not close defect-register rows on planning tests alone.

Qualify the installed standard reader, CLI/RPC/HTTP inventory agreement, three profiles, stale evidence/approval, operator fstab edits, boot/serial/UUID conflicts, restart recovery, credential exclusion and synthetic data preservation in a disposable Debian VM. Bind the exact source and package binaries to the result. Preserve earlier qualified artifacts and report native ARM64, UI and mount execution separately.
