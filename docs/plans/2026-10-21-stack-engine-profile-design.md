# Protected Engine reads in the root reader

The installed reader on `9249c73` now collects genuine catalog facts, but the
Engine socket's protected `openat2` returns `ENOSYS`. The source-bound Debian 252
probe isolates `RestrictSUIDSGID=yes`: disabling only that setting permits the
same syscall, while an explicit syscall allow alone cannot override its filter.
The earlier genuine Docker attempt confirms refusal before socket transport.

## Compatible role profile

Set `RestrictSUIDSGID=no` only in the dormant root storage/stack reader and list
`openat2` explicitly alongside `@system-service`. Keep UID 0 with empty bounding
and ambient capabilities, `NoNewPrivileges=yes`, the actual host mount view,
fixed package worker, closed read-only protocol, protected source/socket traversal,
peer checks and original leases/revalidation. Keep 64 MiB, 256 descriptors,
sixteen tasks, CPU quota, activation policy and all write ceilings unchanged.
Standard and shadow package transformations must preserve those properties.

This is an explicit confinement exception. The profile no longer blocks setting
SUID/SGID file bits. `NoNewPrivileges` prevents privilege gains at exec, but does
not replace that file-creation restriction. The fixed reader accepts no command,
file-write operation or caller-selected executable; trusted root/same-UID writers
remain outside the protected-evidence threat model. The change supplies a kernel
interface required by protected reads; it does not add an effect operation or
claim an equivalent SUID-creation ban.

The [systemd 252 filter](https://github.com/systemd/systemd/blob/v252/src/shared/seccomp-util.c#L2015)
cannot inspect the indirect `open_how` mode and deliberately returns `ENOSYS`.
Adding an allow alone is therefore insufficient. Falling back to an unprotected
open would violate the frozen Engine constructor contract. Granting capabilities
would not remove the inherited seccomp filter. This scoped profile correction is
also consistent with the separately qualified storage-mount role's exception;
it grants none of that effect role's capabilities or mount syscall.

## Verification and package identity

Pin the standard/shadow read-role contract, unchanged privilege/resource limits,
fixed executable/paths and dormant installation in focused reference checks.
Package a distinct immutable candidate. Since this is a unit/profile-only change,
reuse all five verified Debian binaries from `7f35dd5` only after matching every
Rust/Cargo/contract/frontend input hash against that build. Record binary source
and package source separately; any application-source change requires a new
release build. Preserve the prior refused candidate and diagnostic records.

Use a real root Docker daemon with its own Unix listener in a disposable Debian
VM. Verify catalog plus Engine collection, an empty registered stack, replicas,
one-offs and stopped members, source refusal/recovery, no-renewal GETs, socket UID
refusal and restart recovery through the actual RPC/API. Retain the original
64 MiB/256-FD/16-task service limits and record cgroup readings. A missing daemon
or fixture response cannot supply Engine acceptance.

Native ARM64, installed UI ownership, maximum-layout resource/fault qualification,
whole-stack effects, A1/B1 runtime and P05/P06 remain open. No production Pi window
or activation is implied. All work and scratch stay on repository disk, one local
build/test command at a time with the 3 GiB sampled host-memory stop floor.
