# Protected storage target preparation

RW-040 provides a separate root operator command for preparing empty physical mount targets. HTTP guided previews and their approval tokens remain non-executable. Preparation does not mount, unmount, format a filesystem, write fstab, create media directories or change existing data ownership. Write qualification belongs on disposable test hosts before cutover.

The standard installed executor accepts these JSON commands:

```text
limeos-executor storage-target-plan --contract ROOT_OWNED_JSON
limeos-executor storage-prepare-targets --plan ROOT_OWNED_JSON
limeos-executor storage-target-receipt --action ACTION_ID
limeos-executor storage-reconcile-targets --action ACTION_ID
```

Planning emits a new `storage.prepare_targets` version-1 plan with a random action ID and five-minute expiry. Save and review its output as a root-owned regular JSON file before preparation. All input path components must be root-controlled without symlinks; files must be singly linked and at most 64 KiB. Fresh raw UUID/type/serial, connected boot/swap backings, host mounts and original fstab evidence bind the plan. Old HTTP preview JSON cannot execute. The shadow/relocated executor refuses preparation and reconciliation; these commands require the standard installed binary path.

Each selected filesystem must be unmounted. Every existing parent must be protected and in the host root filesystem. Targets must be absent or empty protected directories. Missing parents, nonempty paths, symlinks, foreign ownership, writable ancestors, other mounts and targets beneath other filesystems are refused. Mounted targets hide the underlying directory and cannot qualify. Preparation creates only missing final directories as root:root 0755, then fsyncs them and their retained parents. It never recursively creates parents or changes existing modes/ownership.

The package creates `/var/lib/limeos/executors/storage` as root:root 0700. A descriptor-relative private SQLite WAL journal records the exact plan/digest before any directory effect. FULL synchronization, directory fsync, bounded admission and a process lock protect persistence. Claims on storage configuration, each UUID and each mountpoint enter the same transaction as the prepared receipt. Symlinks, hardlinked/writable auxiliary files, corrupt databases and unsupported journal versions prevent dispatch. Verified preparation releases claims atomically. Any failure or death after dispatch preserves them; retry returns the receipt without repeating creation. An uncertain receipt exits 1. Lookup opens the existing journal read-only.

Explicit reconciliation re-reads the complete inventory and protected empty targets. Unchanged original target evidence resolves as `precondition_changed`; complete safely prepared targets resolve as `verified`. Both release claims. Original existing directories must retain their identities. Partial preparation, changed devices/parents/mounts/fstab or unexpected data leave claims held. Reconciliation updates receipts only and works after dispatch expiry. It never deletes an uncertain path or repeats creation. A new write requires a new plan and root review.

Stable active swap identities enter the inventory fingerprint. Swap files exclude their connected backing graph, including sibling partitions and device-mapper relationships. Swap utilization is ignored because it changes during normal use. Unresolved swap identities and changing evidence fail closed. Readiness and guided fstab previews refuse selected devices backed by active swap.

Remaining RW-040 work includes a newly versioned human-approved core operation with shared locks across dependent container/share/pool jobs, mount/fstab effects, dependency-aware unmount, runtime mount loss and guided React screens. This journal qualifies target preparation only; it is not yet the cross-executor dependency lock authority. Btrfs, FUSE/removable ownership, vendor serial proof, native ARM64 and reference-host qualification remain open. No defect-register row closes on this milestone.
