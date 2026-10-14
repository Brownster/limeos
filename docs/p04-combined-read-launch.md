# Protected combined-reader launch selection

`limeos_executor::read_launch` supplies a fixed-image launch policy for the
[supervised retained collector](p04-read-worker-supervision.md). The library is
unregistered. A separate opt-in combined probe supplies the test seam; no command,
RPC, route, service or package activates it.

`ConfigurationSelection::new(ConfigurationPaths)` admits all four role names:
contract, pools, protection and shares. Names must be distinct canonical UTF-8
absolute paths, at most 512 bytes and 16 components each, without controls,
empty components, dot or dot-dot. Selection establishes no file-content facts.
The genuine worker acquires and revalidates protected configuration through the
existing closed-profile adapter; missing roles never mean empty configuration.

`ProtectedLaunch::prepare(paths)` selects only
`/usr/lib/limeos/limeos-combined-read-probe`. Held no-follow descriptors require
root-owned, nonwritable ancestors and a root-owned, regular, single-link executable
without set-ID or group/other write permissions. The leaf is classified before
a nonblocking read open. Original ancestor identity/permissions and held/fresh
leaf identities are checked around copying; directory timestamps are irrelevant.
There is no caller-selected image, UID, kernel root, numeric cap or clock.

The executable must have a native ELF64 little-endian header with executable or
shared-object type and a size of 64 bytes to 8 MiB. A 64 KiB scratch buffer streams
it into one memfd while hashing. WRITE, GROW, SHRINK and SEAL seals pin those bytes,
including after source mutation or name replacement. The kernel still validates
the complete ELF at exec. `SealedExecutable::capture(&File)` exposes the content
primitive separately: it authenticates no path and cannot construct a protected
launch token. Neither selection nor protected-launch tokens can be deserialized
as authority.

Preparation begins one original four-second window. `ProtectedLaunch::start`
preserves that deadline in the supervisor, including preparation time. The monitor
owns an image lease until cleanup finishes even after the caller drops its image
or ticket. Launch clears inherited environment and supplies only the sealed
digest and four selected paths; the guardian adds its own checked PID for worker
bootstrap. Each role opens its own `/proc/self/exe`, verifies seals and digest,
and retains that descriptor. Worker exec uses the live guardian's proc/fd path,
so manager death cannot leave it depending on the manager's numeric PID.

`limeos-combined-read-probe` lives under tests/support and requires the existing
`read-worker-probe` feature. It is disabled by default and absent from packaging's
explicit executable list. Its default `contract` role performs no host I/O and
emits one final blocked event. The `probe` role uses four bounded environment
names (`LIMEOS_READ_CONTRACT`, `LIMEOS_READ_POOLS`, `LIMEOS_READ_PROTECTION`,
`LIMEOS_READ_SHARES`) and the fixed protected image. Internal guardian/worker roles
require sealed-current-image verification before work; direct on-disk invocation
refuses. The existing standalone probe's combined case remains blocked until
external qualification adopts this separate seam.

The prefix `LIMEOS_RW040_COMBINED_JSON=` carries one complete final JSON event,
built within 264 KiB before output and explicitly flushed. Consumers must require
successful program exit, exactly one final event and the expected status; a
partial prefix is no completion record. An `ok` event requires complete supervised
report transport and confirmed cleanup, then borrows at most 256 KiB of report
JSON. Selection and image metadata describe past observations and executable
bytes; they supply no storage-effect authority.

Local nonroot tests use owned copies of `/usr/bin/true` and a stripped copy of the
opt-in helper (binutils). They prove immutable bytes, active seals, image bounds,
closed launch variables, original ancestor permissions, deadline preservation,
monitor lease ownership and genuine refusal at the public configuration guard.
Private protected-walk fixtures use only an owned scratch root and UID; they
qualify no public root-positive launch. See the
[design](plans/2026-10-14-combined-read-launch-design.md) and
[local acceptance](rewrite-evidence/p04/2026-10-14-combined-read-launch/README.md).

The sealed copy pins executable bytes, not dynamic loader or library contents.
Memfd pages add up to 8 MiB per image to the whole process tree. Installed
memfd/noexec, proc/fd, syscall, capability, namespace and resource compatibility
remain unqualified under the unchanged 256-FD, 64-MiB, zero-swap and 16-task guards.
No executable-memory policy is bypassed. Source preparation and exec can block in
kernel work; termination and latency for those paths remain open. Independent
review, root-positive composition, performance, complete consumers/claims,
actual pool/share state, locks, effects, recovery, P04/defect closure and Pi cutover
remain separate work. The frozen signed ARM64 candidate excludes this slice.
