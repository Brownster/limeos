# Protected combined-reader launch: local acceptance

Qualified runtime `fbd89657fc960f52349a1c1876ddd98678c50e0d`, based on local main `84adb4e`.
The unregistered launcher selects one fixed protected image and four bounded
configuration names. A bounded native ELF copy is sealed in a memfd; the monitor
owns its lease through cleanup and preserves the original preparation deadline.
Each role verifies its own sealed executable. Worker exec uses the live guardian's
descriptor, independently of the manager's numeric PID. See the
[design](../../../plans/2026-10-14-combined-read-launch-design.md) and
[interface guide](../../../p04-combined-read-launch.md).

**690 Rust cases pass:** 669 unit/integration cases and 21 compile-fail doctests.
The 15 new cases and four-case focused rerun are already in that total.
Workspace tests enable `limeos-executor/read-worker-probe`; both fixture binaries
remain disabled by default and absent from the explicit packaging list.
**61 local ARM64 harness tests pass**, plus strict all-target Clippy, fmt,
contract and repository checks. [checks.json](checks.json) records commands,
environments, runtime hashes and complete merged stdout/stderr transcripts.
Documentation changes during validation altered no runtime source.

Real owned ELF tests prove active WRITE/GROW/SHRINK/SEAL seals, unchanged captured
bytes after source mutation/name replacement, native-format/size admission and
successful execution of the immutable copy. Private protected-walk fixtures
refuse wrong owner, writable or hardlinked files, symlink leaves/ancestors,
directory leaves and unprotected roots. Held and fresh directory checks compare
the original identity/permissions, including changed permissions that remain
nonwritable. These private owned-root tests qualify no public root-positive path.

A stripped owned copy of the opt-in helper passes sealed guardian and worker
startup, resource/bootstrap admission and the genuine public nonroot configuration
guard. An earlier image/bootstrap/budget error cannot pass that expected refusal.
Dropping the caller's image still permits exec because the monitor owns the lease.
An owned sleeper regression proves the original shorter preparation deadline
reaches the ticket/monitor without renewal and waits for fixture exit/lease drop.
On-disk internal roles with a valid digest and all four valid paths refuse at the
current-image seal guard. Contract mode emits exactly one
complete final blocked event without host I/O. Configuration names cannot be
missing, duplicated, noncanonical or amplified beyond their closed limits.

The separate supplied combined probe remains unregistered. Its final event is
built within 264 KiB before output, explicitly flushed, and requires successful
exit plus one complete final event. Successful observations require complete
supervised report transport and confirmed cleanup. Image metadata, selected names
and report JSON supply no storage-effect authority. The existing standalone
probe's combined case stays blocked pending external adoption of this new seam.
The implementing agent reviewed the slice; independent review remains pending.

The sealed copy pins executable bytes, not dynamic loader/library contents. Memfd
pages add up to 8 MiB per image to the whole tree; whole-tree peak headroom is
unqualified. Installed memfd/noexec, proc/fd, syscall, capability and namespace
compatibility remain open. No executable-memory policy is bypassed. Source
preparation and exec can block in kernel work; their termination/latency is not
qualified. Guardian SIGKILL and unkillable tasks retain the supervision limits.
Service guards remain 256 descriptors, 64 MiB, zero swap and 16 tasks. Historical
inventory p95 61.401 ms remains above the 20 ms target. No footprint/performance,
P04, defect or cutover gate closes. The frozen signed ARM64 candidate excludes
this slice; no installed artifact or positive root/Docker/storage composition
is qualified here.

Build/cache/general scratch stayed on disk, including debug ELF copies stripped
by local binutils for the owned launch fixture. Existing storage fixtures were
compiled there before small owned, auto-cleaned tmpfs runtime scratch. Their
50 ms samples observed lower-bound peaks of 36864 bytes and
8 files, not exact maxima. No Pi, SSH, Docker, raw block probe or
guest launch was used. Reserved backup and external qualification paths stayed
untouched.

[source-manifest.json](source-manifest.json) binds 320 source, fixture,
contract and script files to the committed runtime. Fixed-timestamp gzip preserves
full log bytes with raw/compressed hashes; [artifact-sha256.json](artifact-sha256.json)
binds the packet. Reproduce with fresh disk logs using the retained helpers;
compile storage fixtures before their tmpfs runtime run.

Next is independent review and external adoption/installed qualification of the
supplied combined seam. Complete physical consumers/claims, actual pool/share
state, locks, effects and recovery remain open.
