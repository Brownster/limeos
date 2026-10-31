# Preserve Btrfs source identity without inferring physical equivalence

The protected-health workspace run exposed seventeen storage-reader failures on
the repository's Btrfs filesystem. An owned native descriptor probe records
mount ID 67, inode device `0:43` and mount-table device `0:35`; live directory
link counts remain one before/after child creation and become zero on detach.
The process reader already accepts positive links correctly; its test wrongly
assumed every filesystem increments directory links. The source resolver's
device-equality check rejects the valid per-subvolume Btrfs device.

This matches [Linux v6.12 Btrfs getattr](https://github.com/torvalds/linux/blob/v6.12/fs/btrfs/inode.c#L7334-L7366),
which reports the root's anonymous device, and Btrfs's documented independent
[subvolume inode namespaces](https://btrfs.readthedocs.io/en/latest/btrfs-subvolume.html#inode-numbers).
Do not weaken device equality globally or equate these numbers for disk effects.

## Read-only source correction

Introduce a private verified-mount-table view borrowing the existing bounded
rows. Construction must compare them against a fresh genuine proc mount table
and preserve its raw digest. Use this view for source path walking and metadata
inspection. Revalidation must construct it only after the original raw digest
and rows match; the existing final table/host/name/lifetime checks remain.
Test-only raw-table entry points must perform the same authentication, so
forged device/type/missing rows still refuse before metadata is accepted.

Keep exact statx/proc mount-ID matching for every component. Keep exact device
equality for ordinary filesystems. An attested Btrfs row may have a different
anonymous inode device; final source metadata must additionally corroborate
the Btrfs kernel filesystem type with descriptor-relative fstatfs. Preserve
the actual inode device, inode and mount ID in the existing source identity;
preserve the mount-table device separately in the existing mount row. The
serialized shape and version do not change. No value is rewritten to pretend
the two identities are equal.

The genuine table token prevents a caller-supplied `btrfs` label from selecting
this exception. Held descriptors plus the final unchanged table prevent a
reused/changed mount from yielding a successful complete observation. Ordinary
link, no-follow/cached-path, local-filesystem, type, positive-link, length and
report/age/work limits remain unchanged. Token validation adds no retained
descriptor or cloned table; it borrows rows and uses the existing bounded raw
table read/comparison before walking. Final Btrfs fstatfs is local metadata and
remains under the existing worker/deadline requirements.

Existing filesystem/dependency/destination joins retain their exact device
equality checks. A virtual Btrfs identity mismatch remains unavailable there;
this change grants no physical backing, UUID, subvolume mapping or operation
authority. Nested/multidevice Btrfs effect binding requires separate typed
evidence and qualification. The source owner remains a read-only metadata owner.

## Qualification

Keep all failing workspace logs and owned probe data. Run all source/process
storage tests on the same Btrfs repository disk, preserving their wrong-device,
missing mount, remote/FUSE, replacement, expiry and resource refusals. Update the
directory regression to assert a live positive count and unchanged identity
through child creation, then zero-link detachment refusal; it must run on both
constant-link Btrfs and varying-link filesystems. Add direct proof that a real
virtual device stays distinct in the source report and remains refused by
physical joins rather than normalized. Run broad workspace/opt-in regressions,
strict Clippy and unchanged package/contracts checks against the final source.

Commit this storage correction separately from the protected-health feature.
Preserve source-bound evidence, repository-disk scratch, one Cargo job/test
thread, 4-GiB starts and the unchanged sampled 3-GiB floor. No mount, format,
subvolume operation, policy/service activation, Docker, Pi or push runs. No A1,
B1, defect, P04 or cutover gate closes from these local metadata tests.
