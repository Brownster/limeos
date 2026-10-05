# Guided storage planning

P04 can inspect existing filesystems and persist a guided setup preview for the single-disk, separate-downloads and protected-pool profiles. The preview contains the exact assignments, readiness waits and generated fstab section. It has no mount, format, fstab write, unmount or queue endpoint. Applying storage changes requires a later operation version, fresh approval, resource locks and durable executor receipts.

The package includes `limeos-storage-reader.service`, a static root service with an empty capability set. Installation creates its independent read-only ceiling but does not enable or start the service. On a disposable test host, start it with `systemctl start limeos-storage-reader`. The reader uses the host mount namespace and direct block-device descriptors; mount namespace sandboxing would invalidate that evidence. The existing host and container executors retain their previous ceilings and service sandboxes. Shadow's separate paths and policy use the same read-only reader.

The core connects to the root reader at `/run/limeos-storage-reader/executor.sock`; `storage_socket` in core configuration can select its shadow counterpart. Kernel credentials authenticate both ends. Each process admits one inventory collection at a time; collection has a four-second deadline inside the five-second RPC deadline. Larger than 48 KiB evidence or unavailable topology/fstab data fails without a partial snapshot. `limeos-executor storage-inventory` prints the same evidence as JSON and uses the same deadline.

An administrator with a current `storage_manage` grant on `storage:configuration` can use these routes:

| Method and route | Result |
| --- | --- |
| GET `/api/v1/storage/inventory` | Fresh disk UUID/type/serial, boot exclusion, mounted paths and inventory digest. |
| POST `/api/v1/storage/plans` | Persist a `StorageSetupInput`: desired contract, inventory digest and timeout of 1–120 seconds. The inventory is collected again and must match the selected revision. |
| GET `/api/v1/storage/plans/{id}` | Read the owner's durable preview without collection, expiry extension or writes. |
| POST `/api/v1/storage/plans/{id}/approval` | Supply the displayed plan digest. Collect fresh evidence, recheck the human session and grants, then store only a hash of the review token. |
| POST `/api/v1/storage/plans/{id}/cancel` | Withdraw the owner's unexpired preview and remove its review token, even if disks are now unavailable. |

All POST routes require the configured Origin and session CSRF token. Preview tokens expire with the five-minute plan and authorize no executable storage request. There is no task-token approval route. Database schema 6 adds bounded, owner-scoped storage plans under the existing SQLite WAL/FULL writer; failed migrations and audit writes roll back. Reads retain the original expiry and survive a core restart.

The planner selects an existing raw UUID, never a caller-supplied `/dev` path. Root/boot/EFI backing disks and aliases, duplicate UUIDs, changed serial/type, bind subtrees, multiple mounts, a different mounted target and nested physical assignments are refused. This slice renders ext2/3/4 and XFS only. Btrfs, NTFS, exFAT and FAT remain represented in the parity contract; complete backing and ownership adapters must be qualified before setup planning accepts them. No filesystem is formatted.

Fstab inspection opens `/etc/fstab` through root-controlled descriptor-relative components and requires one regular, singly linked file no larger than 64 KiB. Options can contain credentials, so only source identity and target fields leave the reader. A SHA-256 digest binds every original byte, including comments and private options. Unresolved local LABEL/PARTUUID/mapper aliases outside the protected root/boot records fail closed until an adapter can prove their identity. Malformed or duplicate ownership markers are preserved and refused.

The generated section uses `UUID=`, fixed filesystem options (`defaults,nofail,nodev,nosuid`) and bounded systemd device/mount timeouts. Literal case is preserved; spaces and backslashes receive fstab escaping. Caller-supplied options, units, commands or fstab text are outside the closed input schema. An unmanaged record owning the selected UUID, raw device or overlapping `/mnt` target conflicts. Pure replacement planning changes only the `BEGIN/END LIMEOS STORAGE V1` section; unrelated bytes remain exact, and an unexpected source digest prevents replacement. Production never calls a write from this slice.

Target preparation, ownership of media leaves, formatting, mount execution, package/profile startup integration, dependency-aware unmount, mount-loss shutdown and the guided React screen remain pending. A successful preview does not prove an unmounted target path is safe to create or mount. Those checks belong immediately before a future effect, under the same locks and receipt barrier. Native ARM64 and reference-host qualification remain separate gates; wybie and the frozen Python build receive no installation or mutation.
