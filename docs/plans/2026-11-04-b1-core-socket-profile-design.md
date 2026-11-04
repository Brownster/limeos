# Core protected socket lookup under the installed service profile

The [B1 boundary packet](../rewrite-evidence/p04/2026-11-04-b1-observation-boundary/README.md)
establishes one genuine independent Core-UID root health read, while the normal
timer remains unavailable with no first settlement. Core opens its fixed reader
socket with `openat2`; its installed unit still enables `RestrictSUIDSGID`.
Earlier reader qualification found that Debian 252 returns `ENOSYS` for this
combination. That is a specific hypothesis, not an observed Core errno.

Use a standalone disposable AMD64 guest and the existing unchanged signed
packages. Install the exact version, start the dormant reader and verify its
fixed root-owned socket. Keep the real Core and reader units, ceilings, policy,
authority database and package payload unchanged. Docker and a new schedule are
unnecessary for a fixed socket descriptor lookup. The prior packet retains the
independent genuine health-read proof.

A fixed guest-only Python probe performs precisely the client's lookup:
`openat2(AT_FDCWD, /run/limeos-storage-reader/executor.sock, O_PATH | O_NOFOLLOW |
O_CLOEXEC, mode=0, RESOLVE_NO_SYMLINKS | RESOLVE_NO_MAGICLINKS)`. It records errno
or descriptor metadata, actual UID/groups, selected process security fields and
descriptor/task limits. It sends no socket request, touches no Core data and
accepts no caller-selected path, operation, UID, syscall or output filename.
It refuses outside the disposable hostname, AMD64 architecture and actual Core
UID. A direct Core-UID invocation establishes the DAC/visibility control.

Copy the entire installed standard Core unit into four separately named probe
units. Change only `ExecStart` to the fixed probe, `RuntimeDirectory` to an owned
probe directory and `Restart=no` to prevent diagnostic loops. Preserve every
other original directive and bound startup/completion observation. Variants are
baseline, only `RestrictSUIDSGID=no`, only explicit `openat2` in the syscall allow
list, and both changes. Record exact rendered unit bytes and effective systemd
properties. The original Core unit is never reloaded with different content or
restarted; prove its unit hash, live PID and selected database authority counts
unchanged across the matrix. New probe units are stopped/removed before whole-VM
disposal. No production machine is accessed.

This isolates the profile hypothesis if the direct control and unrestricted/
combined variants open the exact original socket, while baseline/explicit-only
return `ENOSYS`. Confirm successful descriptor identity against independent
root-owned socket metadata and preserve all failed attempts. Native cases check
whole-profile preservation, closed variant inputs, probe refusal before syscall,
descriptor cleanup and the expected matrix; they cannot establish installed
seccomp behavior. Freeze exact source/package/fixture identities and run one
canonical 1,024-MiB guest under the existing host admission/memory guard.

If confirmed, the narrow product correction is the Core profile exception
`RestrictSUIDSGID=no` plus explicit `openat2` allow. Preserve the real unprivileged
UID/groups, empty capability sets, `NoNewPrivileges`, namespace/filesystem/network
restrictions, current identity/grant/CSRF checks, protected descriptor/peer checks,
original deadlines, service resources and restart behavior. This explicitly
removes the file SUID/SGID bit-setting restriction. `NoNewPrivileges` prevents
privilege gains at exec; it does not recreate that file-bit restriction. Core
already writes its owned private state; do not describe confinement as unchanged
or grant a capability to bypass the filter. Do not replace `openat2` with an
unprotected lookup. Apply no product exception until the matrix proves the
hypothesis.

A profile-only correction may reuse verified Debian executables/UI only after
matching every application/Cargo/contract/frontend input hash to the original
build, recording package and executable source separately. Produce a distinct
signed local package; verify standard/shadow transforms and existing package
checks. Then run the ordinary B1 scenario without a Core override or probe unit:
require genuine first incident/publication, original identities across actual
Core SIGKILL, same-container recovery, acknowledgment/pause and second restart.
The matrix is not a substitute for normal timer acceptance.

All scratch stays on repository disk. Serial commands retain 4-GiB native and
4.5-GiB VM admission, the sampled 3-GiB stop floor and unchanged guest/service
limits. Normal B1 API/UI/restart, remaining sources, other A/B/P04 parity,
current ARM64/Pi, package return/footprint/soak and P05/P06 remain open unless
their own source-bound evidence establishes completion. No Pi, external send or
publication is authorized by this diagnostic.
