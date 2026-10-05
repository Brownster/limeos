# P04 storage verification and readiness

This first host adapter inspects storage and waits for assigned filesystems. It does not mount, unmount, format, edit fstab or start a stack. Those operations still require the durable approval path, independent executor ceilings, fresh dependency evidence and protected resource locks. A successful snapshot cannot authorize a later effect.

The packaged `limeos-storage-ready.service` is static and dormant. Installation does not create its plan, enable it or add it to boot. Later storage orchestration will supply a root-controlled `/etc/limeos/storage-wait.json` using the generated `StorageMountWaitPlan` schema. Device entries come from the validated contract's assignments; media/download/config/backup subdirectories are not waits. The deadline must be 1–120 seconds. The unit has a separate 125-second startup watchdog, no restart policy and no implicit mount dependency.

For diagnosis, run as root:

```sh
/usr/lib/limeos/limeos-executor storage-check --plan /etc/limeos/storage-wait.json
/usr/lib/limeos/limeos-executor storage-wait --plan /etc/limeos/storage-wait.json
```

Both commands use the same checks. `storage-check` takes one fresh snapshot; `storage-wait` retries only absent mounts/devices until the plan deadline. Unsafe paths, wrong identity and missing evidence fail immediately. Standard output contains a JSON snapshot on success. Standard error contains a bounded error code/message on failure. Exit codes are 0 for verified, 1 for unsafe/unavailable/not-ready, 2 for invalid input and 3 for an expired deadline. `--help` and `--version` are available. No environment variable, shell command or caller-supplied executable controls the probes.

All plan and mount path components are opened relative to retained directory descriptors with `O_NOFOLLOW`. They must be root owned and not group/world writable. Plan reads are regular-file only and limited to 64 KiB. Mount-root checks use descriptor `statx` mount IDs and device numbers, rather than string prefixes or directory existence. Media ownership below the mount root is outside this readiness check.

Fresh sysfs device numbers, partition parents and slave relationships identify backing disks. All connected devices beneath root, `/boot` (including firmware/EFI) and `/efi` are excluded, including sibling partitions and device-mapper aliases. Every non-virtual block node is probed directly by `/usr/sbin/blkid -p` through a retained device descriptor, with no persistent cache. A duplicate filesystem UUID anywhere in that set fails closed. An optional serial must match the disk ancestry; multiple different backing serials are ambiguous. UUID/type are probed again for each selected filesystem. Topology, mount-table and namespace changes during collection fail verification.

Verification must run in PID 1's mount namespace. The readiness unit deliberately omits mount-namespace sandbox settings such as `ProtectSystem`, `ProtectHome`, `PrivateTmp` and `ProtectKernelTunables`; applying them would hide the host mounts. It retains an empty capability set, fixed package-owned executable, syscall/process/memory limits and no network socket families beyond Unix. Linux must provide `statx` mount IDs (5.8 or later). The existing long-lived host observation service retains its original sandbox and does not execute this verifier.

Ext2/3/4, XFS, VFAT, kernel exFAT and kernel NTFS can be verified when the descriptor and mounted filesystem agree. Btrfs and FUSE-backed NTFS remain closed until their complete backing-device mappings are implemented and qualified. The wire contract still preserves those legacy filesystem choices; rejection is explicit rather than guessing a backing disk. Unknown root topology, missing device permissions, hotplug, unsupported signatures and unavailable probes also fail closed.

Qualification uses only empty virtual disks attached to a disposable Debian guest. Native ARM64, guided disk assignment, API/UI, mount/fstab plans, unmount dependency shutdown and runtime mount-loss monitoring remain outstanding. MNT-001 remains open until the complete profile startup path uses and qualifies this mechanism.
