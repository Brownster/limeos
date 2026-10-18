# A3b remote-mount preservation inventory

This inventories the frozen SSHFS, rclone S3 and separate seedbox mount flows at
pi-health `80593b29443ea9b81eb60ed4e5c3c3ac7a236204`. It supplies the missing A3b
preservation/contract input on `engineer/p04-a-assistance`, following candidate
`59f39873ca8e127a644e73e123829c9846a98095`. It adds documentation and source
evidence, with no remote Rust operation, import, credential authority or installed
qualification. All 30 rows remain unimplemented/unqualified; no retirement is
accepted. Deployed usage/configuration has not been collected here.

The [matrix](rewrite-evidence/p04/2026-10-18-remote-preservation-inventory/inventory.json)
records intent, observed frozen behavior, proposed replacement, source anchors
and required acceptance for every row. Its
[coverage](rewrite-evidence/p04/2026-10-18-remote-preservation-inventory/coverage.md)
maps all 56 selected public methods, helper commands and API handlers through 81
anchors. The [extraction](rewrite-evidence/p04/2026-10-18-remote-preservation-inventory/source-extraction.json)
contains the exact literal schemas, declared defaults, saved-field expressions,
helper dispatch and route declarations. These are static facts, not proof of
actual effects, authorization, complete deployed inventory or recovery.

## Preserved scope and actual sources

The generic plugin registry supplies SSHFS (enabled by default) and rclone
(disabled). Rclone's schema and helper support S3 and S3-compatible endpoints;
UI mocks naming gdrive/dropbox are not evidence of those backends. The fixed
seedbox service is a third flow, outside the generic plugin JSON. It cannot be
discarded as a duplicate SSHFS implementation.

The separate `LegacyStorageCapabilityAdapter` enumerates MergerFS/SnapRAID, not
these remote providers. A category-to-page mapping does not register remote
runtime parity. The generic base class mentions NFS/SMB in a comment, which also
does not establish an implemented provider. New NFS/CIFS remains outside this
assignment unless separately accepted. Imported unknown providers/settings must
remain visible and unresolved rather than be silently discarded.

| Flow | Saved/application sources | Root/effective sources that must also be reconciled |
| --- | --- | --- |
| SSHFS | `<config_dir>/sshfs_mounts.json`, `mounts` envelope | `/etc/sshfs/mounts.json`; per-id password files; generated mount/automount units; actual namespace/unit/process facts |
| Rclone | `<config_dir>/rclone_mounts.json`, `mounts` envelope | `/etc/rclone/mounts.json`; `/etc/rclone/rclone.conf` sections; generated service; actual namespace/process/cache facts |
| Seedbox | Runtime `seedbox_mount.json`; fixed `/mnt/seedbox` | `/etc/sshfs/seedbox.pass`; `mnt-seedbox.mount`/`.automount`; actual namespace/unit/process facts |

One successful source is insufficient. Missing, denied, malformed or conflicting
records are coverage findings, not default empty configurations. Plugin JSON,
helper JSON, generated units, credentials and runtime state can describe
different settings. Disabled saved configurations still need preserved identities
and dependency/credential consumers.

## Field/default inventory

These are frozen source defaults, not a new canonical Rust schema. Omitted data
in one store must not be filled from defaults when another source may carry the
operator's intent or an effective value.

| Flow | Exact advertised/submitted fields | Defaults and source gaps |
| --- | --- | --- |
| SSHFS | Required `id`, `name`, `host`, `username`, `remote_path`, `mount_point`; also `port`, `auth_type`, `password`, `ssh_key_path`, `enabled`, `options` | Port 22; auth password or key, default password; enabled true. Root record omits auth type/key path. Plugin strips password before saving. Helper always uses sshpass/password file, ignoring advertised key mode. |
| SSHFS options | `reconnect`, `compression`, `allow_other`, `server_alive_interval`, `server_alive_count_max` | Advertised true/false/true/15/3. Helper consumes reconnect/compression but always adds allow_other and uses fixed 15/3 on reconnect. Saved requested options do not prove effective options. |
| Rclone | Required `id`, `name`, `backend`, `bucket`, `mount_point`; also `provider`, `access_key_id`, `secret_access_key`, `region`, `endpoint`, `enabled`, `options` | Backend s3 by default; provider AWS; region us-east-1; enabled false; S3-compatible requires endpoint. Plugin strips only secret key, retaining access key ID. Root record omits both credentials and all options. |
| Rclone options | `vfs_cache_mode`, `read_only`, `allow_other` | writes/false/true. Helper adds directory cache 5m, poll 1m and restart delay 5s. These effective flags live in the generated service rather than root metadata. Cache/space/ownership bounds need their own accepted resource profile. |
| Seedbox | Submitted enabled/host/username/port/remote_path/password; mountpoint supplied by the fixed service | Default disabled, empty host/user/remote path, port 22 and fixed mountpoint. Enabled validation requires password and absolute remote path. Password is not saved in application metadata. Disable can replace metadata with the fields/defaults in its request. |

SSHFS's plugin ID pattern allows lowercase letters, digits and hyphens. Its
helper accepts uppercase/underscore too. Helpers sanitize unit names by replacing
non-alphanumeric characters with hyphens: `nas_one` and `nas-one` collide. IDs
must remain exact during inventory/migration; a normalized display ID supplies no
safe authority to merge, overwrite or delete objects.

Generic SSHFS ID `seedbox` and the dedicated seedbox flow reference the same
`/etc/sshfs/seedbox.pass`. They may have distinct unit/target identities. Every
consumer must survive reconciliation and any removal policy. Rclone removal also
needs explicit section/credential retention rules: base plugin removal only
deletes its display row, while the helper removal path leaves the INI section.

## Source-observed defects and required replacements

Rows RM-008/009/017/022 distinguish enable, disable and deletion. Generic enable
shortcuts send `enable` and insufficient configuration, while helpers consume
`enabled` plus full fields. SSHFS disable calls removal and can erase units/root
metadata/password while retaining the plugin row. Proposed preservation of
settings on disable requires explicit contract acceptance; this inventory does
not silently redefine those semantics.

Rows RM-010/018/021/025 cover partial publication and cleanup. Helpers update
multiple files before application metadata, ignore some reload/disable/stop
results, and lack the shared recovery journal. A failed stop cannot justify
deleting a visible row or returning a successful cleanup receipt. A later
metadata failure after accepted effects needs reconciliation, not blind replay.

Generic SSHFS names its mount unit from an ID. For example, ID `nas` produces
`sshfs-nas.mount`, while target `/mnt/nas` requires its path-derived identity.
Systemd requires mount unit names to reflect the mountpoint path. This is a
source/documentation inconsistency, not an installed-host test result.
[systemd.mount](https://www.freedesktop.org/software/systemd/man/latest/systemd.mount.html)

Read status must not probe a remote directory implicitly. The frozen SSHFS
status performs unbounded `listdir`, and turns permission errors into connected
status. Access to an active automount path can activate its corresponding mount;
systemd also warns about network ordering dependencies on automount units.
Use protected namespace/unit metadata for GET status. Any access-health probe
needs a separately accepted bounded operation. Installed versions and actual
failure/cleanup behavior still require qualification.
[systemd.automount](https://www.freedesktop.org/software/systemd/man/latest/systemd.automount.html)

Rows RM-026/027 keep package/provider lifecycle separate. The root helper may
install packages during configure; use B's accepted APT/prerequisite operations
instead. Its dependency check accepts fusermount or fusermount3, while the
generated rclone unit fixes `/bin/fusermount`; the plugin checks fusermount alone.
Do not infer installed compatibility. Generic plugin reads also save merged
defaults, and enabling can import mounts. Replacement GETs must never normalize,
write, install, import or activate anything.

Rows RM-011/024 preserve literal paths and failures through native UI. Existing
provider output uses `mount_point`, while the UI reads `mountpoint`/`target` and
loses that path. HTTP 400 is collapsed to unsupported. Typed fields must preserve
paths, distinguish provider failure from supported emptiness, and expose unknown
source/credential/access state. Detection must handle escapes/IPv6/colons without
assuming a usable key or safe ownership. Discovery and approved import must be
separate.

## Exact handoffs still required

The [A assignment](plans/2026-10-14-engineer-p04-storage-backup-runtime.md) requires
agreement before new remote domain/reader names and central contracts. This work
does not add `remote_mounts.rs`, shared exports, credentials, SQL or a parallel
resource registry. The integrator still needs to accept these seams:

1. A complete protected inventory response retaining each original source's
   lifetime, identity, coverage, bounds, requested/effective settings and
   conflicts. Raw unit text and saved JSON are data, never effect authority.
   Limits must cover sources, records, unit/config bytes, FDs, memory, tasks,
   cache/disk space and whole-worker collection, with explicit overflow findings.
2. Exact credential references/versioning and host-trust policy for passwords,
   accepted key mode and S3 credential pairs. Omitted/unchanged/replaced/removed
   credentials must be distinct. Bind every shared consumer; no raw path supplied
   by a client selects a root credential or configuration.
3. A genuine FUSE/process/namespace/configuration identity contract plus complete
   sorted target/remote/unit/credential/stack/share/backup claims. A1's ext4-only
   admission remains closed to these identities. Agree dependency loss,
   quiesce/verified stop and recovery interfaces with A2/B before effects.
4. Registered configure/update/enable/disable/remove/mount/unmount/import/probe
   operation semantics and versions, current principal/grants, revision/plan
   binding, fresh approval, durable idempotency, supervision and atomic
   before-effect journal/receipts. Actual unit/helper/mount verification and
   uncertain-effect barriers govern completion, not a manager return code.
5. Centrally generated status/readiness/error DTOs, route registration and native
   UI. Configuration actions use the accepted POST/CSRF/durable-job contract;
   stale permissions/revisions, reconnect, lost responses and partial discovery
   stay visible. Adjacent media/startup controls retain their A2/B acceptance.

Seven [synthetic reconciliation cases](rewrite-evidence/p04/2026-10-18-remote-preservation-inventory/reconciliation-cases.json)
make the source/identity conflicts concrete. They are future acceptance inputs,
not executed tests or an importer. Each matrix row has its own required scenario;
the qualification owner still needs installed configuration/lifecycle/API/UI,
disconnected endpoint, bad credentials, stale identity, full dependencies,
concurrent backup/edit, process loss, restart and cleanup evidence. No fixture
authorizes a production Pi or another host test window.

The four previous assistance leaves retain their source-bound local results at
`8b31f990`: 331 Rust checks, 129 frontend cases and the actual UI build. This
documentation-only change repeats no Cargo/frontend suites and earns no new
runtime acceptance. A1's installed gate, A2 lifecycle/loss, A3 effects, A3b runtime
and A4/A5 backup/restore remain open. The inventory latency gate remains failed
at 61.401 ms p95 against 20 ms; P04/P05/P06 cutover order is unchanged.
