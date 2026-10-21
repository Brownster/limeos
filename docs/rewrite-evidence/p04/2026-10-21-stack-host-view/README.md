# Stack catalog host-view correction

Source `7f35dd5c7b1d460fce860bff511329e19b6f130f` replaces the denied PID 1
namespace-link open with a kernel mount-ID proof tied to the retained root
descriptor. Genuine catalog reads now pass through the installed worker, RPC and
authenticated API with UID 0 and an empty capability set. **Full stack/B2 and Pi
cutover acceptance remain open.**

The [design](../../../plans/2026-10-21-stack-reader-host-view-design.md) describes
the proof and bounds. Both fixed proc tables must be genuine UID-0-owned procfs
files. Their unique root entries must agree with the held root's required mount
ID and device, and with each other's mount-root bytes. Acquisition retains and
rechecks the current NSFS owner. Existing file/root identity, ownership, lease,
cancellation and final revalidation stay in force. The table, row and count
bounds are 64 KiB, 4 KiB and 512 respectively.

## Source-bound checks

| Check | Result |
| --- | --- |
| Native container crate | 356 unit tests and seventeen compile-fail doctests passed, including seven new host-view cases |
| Strict container Clippy | All targets passed with `-D warnings` |
| Fresh Debian release build | All five binaries built from clean `7f35dd5`; input hashes match before/after and across the container bind |
| Standard/shadow packages | Both assembled and signed as `0.4.5+stack.2`; immutable package/binary/UI hashes are in the candidate record |
| Installed standard catalog | Six registered-path checks passed, listed below |
| Profile diagnostic | The successful disposable probe isolates the `RestrictSUIDSGID` conflict; the initial fixture startup failure is preserved |

Native tests and Clippy ran on the feature's uncommitted bytes while HEAD was the
design commit `c44a7ea`. Their before/after input hashes bind those exact bytes;
they match the subsequently committed source and Debian build inputs. This is
focused crate qualification, not a new full-workspace, frontend or ARM64 result.

The Debian build used the same offline Rust 1.88 image as the preceding attempt,
one Cargo job and a 2 GiB container limit. It exited zero without OOM, preserved
source hashes and removed its owned container. Build, package assembly and every
VM command used the sampled 3 GiB host stop floor. Work and scratch stayed on
repository disk. Each owned VM was destroyed after its terminal result. No Pi,
production service or frozen Python source was touched.

## Installed catalog behavior

The standard package is installed from a signed local test repository in a fresh
768 MiB Debian VM. The reader stays dormant until explicitly started by the
fixture. The test provisions one protected catalog registration, enrolls a real
administrator and exercises the actual installed paths:

1. The sealed worker produces genuine fresh catalog data through root RPC and
   authenticated HTTP. Foreign UID access to the same socket is refused.
2. Twenty HTTP reads preserve the original catalog sample timestamp in root RPC.
3. Making the catalog group/other-writable refuses new acquisition and retains
   explicitly stale facts with unknown runtime/counts.
4. Restoring protected permissions obtains a new genuine sample.
5. A test-only `PrivateMounts=yes` drop-in creates a real private namespace; the
   installed path refuses collection. The drop-in is then removed.
6. Returning to the host view and restarting the reader and Core recover genuine
   catalog data through the same authenticated API.

The fixture deliberately installs no Docker daemon. Engine data stays unavailable,
runtime stays unknown, counts stay withheld and files stay `not_inspected`.
This test does not replace the pending real Engine/member scenario. Existing
service capabilities, syscall settings, 64 MiB/256-descriptor/sixteen-task limits
and activation policy are unchanged by the correction.

Recorded service cgroup peaks are 2,994,176 bytes before restart and 3,129,344
bytes in the final service instance, with no recorded memory/OOM or PID-limit
events. Restarts reset cgroup accounting. These are catalog-only phase readings,
not complete maximum-layout resource or descriptor/task-peak acceptance.

## Confirmed independent Engine conflict

A separate AMD64 kernel probe compares the exact reader profile against three
single-setting/control variants, retaining zero effective/bounding capabilities
and `NoNewPrivileges=1` in every unit:

| Profile | `openat2` errno |
| --- | --- |
| Outside the unit | 0 |
| Unchanged reader settings | `ENOSYS` (38) |
| Only `RestrictSUIDSGID=no` | 0 |
| Only explicit `openat2` allow | `ENOSYS` (38) |
| Both changes | 0 |

This identifies the installed systemd 252 SUID/SGID restriction as the source of
the refusal. Its [upstream implementation](https://github.com/systemd/systemd/blob/v252/src/shared/seccomp-util.c#L2015)
blocks the indirect `openat2` argument with `ENOSYS`. The production setting was
not changed here. The probe also confirms that empty-capability UID 0 can read
both mount tables while PID 1's namespace link is denied; a real `unshare`
changes the root mount ID from 27 to 294. The installed application subsequently
verifies the private-namespace refusal using the new proof.

The initial probe stopped at test-unit startup before running any in-unit
diagnostic. The fixture had not provisioned the package's service group because
it intentionally did not install the application. The corrected fixture supplies
that disposable group; no production assertion or guard was removed.
The probe-2 directory also inherited two probe-1 diagnostic text files when its
scripts were copied. The packet labels those byte-identical files
`inherited-probe-1-*`, with explicit original-path/hash records in the manifest.
They are not outputs of the passing probe; its result JSON and guarded log are
retained separately.

## Evidence and remaining work

`manifest.json` binds compressed original records, helper/guest snapshots,
the source patch, candidate hashes and signed public repository metadata.
Each entry records compressed and original SHA-256. `artifacts.sha256` covers
the final packet. The old namespace/socket refusals remain in the
[preceding packet](../2026-10-21-stack-installed-read/README.md).

Resolve the Engine confinement conflict with an explicit compatible profile
decision, then freeze a new candidate and qualify real catalog/Engine collection,
replicas, stopped members, one-offs, source failures, installed UI ownership,
worker/guardian/manager faults and maximum-layout resources. Shadow installation,
native ARM64, A1 admission, B1 observation production, whole-stack effects and
P05/P06 migration/return gates remain open. A passing catalog correction does not
close any of those requirements.
