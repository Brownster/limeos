# Dedicated storage mount profile

The standard package contains a dormant `limeos-storage-mounts.service` for the
initial `storage.mount@1` flow. Installing the package does not provision a mount
ceiling, grant a Core permission, enable the unit or start it. The shadow package
omits the effect unit. No installed effect or confinement qualification is
claimed by this source profile.

The service creates `/run/limeos-storage-mounts/executor.sock` itself as root,
group `limeos-host-access`, mode `0660`, beneath a root-owned `0750` runtime
directory. It does not use socket activation. Its fixed executable is
`/usr/lib/limeos/limeos-executor`; the guardian executes a sealed copy of that
protected image and the only mount helper image is `/usr/bin/mount`. Package
payload ownership is root, executable modes exclude group/other writes, and the
builder rejects writable or symlinked execution paths and a changed mount unit
command. Guest qualification must also inspect `/usr/bin/mount`, its interpreter
and shared libraries, and all their ancestors for root ownership and write modes.
No executable or policy is loaded from Core's writable state directory.

## Opt-in configuration in a disposable guest

This procedure describes a candidate for a separately authorized disposable
guest run. It must not be applied to the development host or Pi. The guest needs
a separate data disk containing an existing, uniquely identified ext4 filesystem;
boot/root backing, active swap, loop files, sibling/holder aliases and unsupported
device stacks refuse. This flow performs no format, directory creation, ownership
change or fstab update. Prepare the existing empty root-protected mount target
using the separately reviewed target workflow first.

The unit requires all five protected files before it can start:

| Path | Role |
| --- | --- |
| `/etc/limeos/system-policy/storage-mounts.json` | Independent root ceiling |
| `/etc/limeos/storage-contract.json` | Managed target identities and paths |
| `/etc/limeos/pools.json` | Complete declared pool configuration |
| `/etc/limeos/protection.json` | Complete declared protection configuration |
| `/etc/limeos/shares.json` | Complete declared share configuration |

All files and their ancestors must be genuinely root-owned and protected from
group/other writes. Missing files never imply empty configuration. The selected
Data/ext4 device must be absent from configured consumers, and its current
physical, process, namespace, target and consumer evidence must pass the genuine
producer. The initial profile supports at most four running containers, sixteen
declared destination mounts, 256 host tasks and eight mount namespaces. This is
not wider-host parity; inaccessible, changing or excess proc evidence refuses.

`/usr/share/doc/limeos/examples/storage-mounts.example.json` is a disabled example
with no targets. Its sample UID is not authority and the package never copies it
into `/etc`. In the guest, the root operator must provision an independent ceiling
using the actual `limeos-core` UID and an exact target ID, filesystem UUID and
mountpoint already present in the protected contract. Leave `allow_mount=false`
until the concrete guest disk and test scope have been reviewed. There are no
wildcards or automatically generated write grants.

The companion `storage-mounts-core.example.json` is a merge fragment, not a full
Core configuration. Preserve the guest's existing `/etc/limeos/core.json` fields
and explicitly set:

```json
{"storage_mount_socket":"/run/limeos-storage-mounts/executor.sock"}
```

Validate the complete Core configuration before restarting Core in that guest.
The standard default already uses this path; recording it explicitly makes the
qualification input unambiguous. The shadow configuration instead uses
`/run/limeos-shadow-storage-mounts/executor.sock`, where no effect service is
packaged. Root ceiling provisioning does not alter current Core grants,
authentication or approval requirements. Starting the dormant standard unit is
an explicit guest operator action after profile review; no package dependency or
Install section starts it.

## Privilege and namespace contract

This is a separate effect role. Existing storage readers retain their original
units and capability bounds. The new role has exactly
`CAP_DAC_READ_SEARCH CAP_SYS_PTRACE CAP_SYS_ADMIN` in its bounding and ambient
sets. The genuine producer and guardian check the effective set. These permit
protected host proc inspection and the fixed mount effect; they are not granted
to Core. No shell or caller-selected program is accepted.

Runtime compares actual current and guardian mount namespace/root identities
with independently opened PID 1 anchors. The role disables filesystem sandbox
settings that would produce a private mount namespace, including ProtectSystem,
ProtectHome, PrivateTmp/Devices and ProtectControlGroups. `PrivateMounts=no` alone
would not establish host visibility. Additional bind/path sandbox directives or
hidden proc settings are unsupported and must refuse through genuine evidence.
There is no setns fallback or test authentication substitution.

`RestrictSUIDSGID=no` is specific to this role because the existing confinement
findings showed openat2 blocked by that setting. The syscall filter preserves
`@system-service` and explicitly lists `openat2 mount pidfd_open
pidfd_send_signal prctl setsid memfd_create`. The guardian establishes its session
and subreaper before READY; a real durable identity and fresh one-use journal
permit precede GO. The helper uses fixed ext4/rw/nosuid/nodev arguments with
`--internal-only`, no mtab write, and no external mount helpers. Other mount APIs
are not added speculatively; unsupported helper behavior must fail in the guest.

The process keeps the existing `LimitNOFILE=256`, `TasksMax=16`, `MemoryMax=64M`
and `MemorySwapMax=0`. The source's conservative reservation is 225 FDs within
the existing 226 worker model, with baseline, transient and guardian ownership
charged before acquisition. Actual peak descriptors, RSS, task count and latency
remain unqualified. The historical storage-inventory latency budget remains
failed. Socket-activated Docker peer identity, openat2/proc access and capability
persistence across guardian/helper exec must be observed in the actual guest.

## Stop, upgrade and recovery

The unit uses `Restart=no`, `KillMode=control-group` and `SendSIGKILL=yes`.
Package removal or upgrade stops it before replacing executor code. The prerm
requires the actual group-wide stop settings, a successful stop and inactive
state; any surviving cgroup must report no populated descendants. Unsupported
cgroup layout, timeout, failed stop or blocked descendants abort replacement.
These checks control package teardown only and never mint a helper receipt.

The optional-service LoadState query is checked normally. Masked, error, unknown
or failed-manager states abort replacement. A missing unit may be skipped only
when it is genuinely not-found, inactive, has no manager cgroup and has no
remaining standard unit cgroup in the supported unified hierarchy. An absent
mount or empty cgroup never supplies a recorded helper cleanup fact.

The root mount journal remains in `/var/lib/limeos/executors/storage/mounts.sqlite`
under the existing root-only storage state directory. It shares `targets.lock`
with target preparation. Removal preserves journals, claims, users and policies.
Neither a stopped unit nor an absent mount is converted into a durable Reaped or
no-effect fact. Pending/Unknown work retains its barriers until a separately
reviewed genuine termination/reconciliation path proves the required facts.

Guest scenarios must include the real separate-disk success path, existing
filesystem-root UID/mode differing from the prepared underlay, all refusal
cases, per-TID unshared descriptor tables, literal-space/deleted mapped files,
missing trust/capabilities, private namespaces, wrong/expired GO, parent death
before and after GO, helper/descendant termination, D-state uncertainty, exact
fresh mount flags, crash replay and optional-service package teardown. Native
checks, static unit contracts and these proposed scenarios do not close the
installed, resource, recovery, cutover, production-policy, Pi or footprint gates.
