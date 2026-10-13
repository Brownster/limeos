# Basic Rust cutover readiness

On October 12 the operator said that a replacement can happen once the Rust
rewrite supplies enough basic functionality. The Pi remains the household's
daytime media server. This changes the release priority; it does not declare a
feature ready or authorize two applications to mutate the host together.

Tonight's candidate is source/fixtures `016ba49`, schema 9, package
`0.4.8+arm64.2`. The [qualification runbook](2026-10-12-arm64-integrated-qualification-runbook.md)
uses disposable native ARM64 KVM guests on `holly@192.168.0.45`, with an explicit
window ending October 13 at 07:00 Europe/London. The operator additionally
approved retaining dormant QEMU prerequisites for subsequent tests. No Rust
package was installed on the production host in this run. The
[overnight report](../rewrite-evidence/arm64/2026-10-13-current-016ba49-memory-guard/README.md)
records successful deadline supervision and healthy teardown, but both native
builds stopped at the host swap guard. Installed qualification remains open.

## Candidate first-release scope

The smallest currently implemented daily-use surface is login, host/resource
overview and history, container inventory/logs, and approved container
start/stop/restart with durable jobs. These are a candidate cut line for review,
not an assumption that all other current functions are unused.

| Function | Current implementation | Evidence still needed for this installation |
| --- | --- | --- |
| Login, imported password hashes, durable sessions | Delivered; signed-package and source regressions exist | Rehearse import of a protected copy of the actual user state; validate account access and rollback |
| Host/resource overview, history, SSE | Delivered in the native dashboard | Read-only shadow comparison against this Pi's real disks, pools and 14 running containers; decide how legacy history is retained |
| Container logs and approved start/stop/restart | Delivered with narrow executor, receipts and durable jobs | Current native package/crash qualification, then a rehearsal with copied stack state, intended resource grants and mounted-data safety preconditions |
| Storage inventory and guided previews | Read-only adapters and typed previews exist | Current native collection, real-layout comparison and a clear unavailable result for unsupported evidence |
| Live mounts/fstab, dependency-aware unmount and mount-loss shutdown | Not delivered as a complete registered flow | Protected physical/namespace composition, complete claims, effect-time validation and recovery; these remain blockers for any storage-changing release |
| Pool, SnapRAID and Samba configuration | Pure planners and retained evidence libraries are integrated | Protected runtime collection and approved execution; planner tests do not qualify live configuration changes |
| Backup and restore | Inspection, custody, staging, private preparation and before-images exist | Encrypted backup creation, production limits, service/DB coordination and managed-destination restore/recovery; BKP-001 remains open |
| Catalog/Compose deployment, updates, schedules and notifications | Partial foundations or pending | Agree which are required daily, then implement and qualify each preserved flow; existing schedules cannot disappear silently |
| Native assistant | Pending | It can be deferred from a proposed basic first release if the operator accepts that scope; no assistant parity is claimed |

## Work before an activation decision

1. Finish current native package, genuine schema-8 upgrade, interruption and
   footprint qualification. Record failures and the distinction between 4 KiB
   guest pages and the production kernel's 16 KiB pages.
2. Inventory the functions and schedules actually used on this installation.
   Agree a short preserved/deferred/retired list. Existing media applications,
   mounts, library paths and numeric ownership continue to work independently.
3. Supply a read-only shadow package with a separate port, users, paths and
   enforced read ceilings. Compare real observations within an agreed window;
   remove it cleanly before enabling a replacement's write ceiling.
4. Build and rehearse migration and return using a protected copy of the real
   configuration, users/hashes, stack roots, storage contract and relevant
   history. Verify failed activation and offline recovery. Test repository keys
   are not production release keys.
5. Present the exact release artifacts, migrated-state report, remaining feature
   limits and rollback commands for the activation decision. Stop/disable the
   frozen build's writers before the Rust build gains write authority. Preserve
   the old checkout, runtime state and every media/storage path through soak.

The next integrator work remains bounded protected RW-040 collection and
dependency/claim composition. A separate completed pure dependency-graph branch
must be reviewed and integrated independently; it is outside tonight's frozen
candidate. Backup-bound recovery remains owned by its existing worktree.
