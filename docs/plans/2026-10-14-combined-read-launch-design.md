# RW-040 protected combined-reader launch selection

Base: local main `84adb4e`. Supervision mechanics accept trusted Rust commands,
but production image/configuration selection is not integrated. Add an
unregistered fixed-image policy and an opt-in standalone combined probe. No
runtime service/command/RPC/route/package change or privileged local collection.

## Executable bytes and provenance

Use only `/usr/lib/limeos/limeos-combined-read-probe` for protected launch. Walk
from `/` with held no-follow O_PATH descriptors, requiring root-owned nonwritable
directories and a root-owned, regular, single-link executable without set-ID or
group/other write permissions. Classify the leaf before a nonblocking read open;
compare held/fresh identities and source size/mtime/ctime around copying. Missing,
symlink, writable, replaced, changed or oversized sources refuse.

Copy at most 8 MiB of native 64-bit little-endian ELF into one memfd, streaming
through 64 KiB scratch while hashing. Reject scripts, wrong architecture and ELF
type. Seal WRITE/GROW/SHRINK/SEAL and make the file owner read/execute only. A
descriptor alone would not prevent in-place modification; immutable copied bytes
avoid both that race and name replacement between admission and exec.

Expose the bounded `SealedExecutable` content primitive separately from protected
provenance. It accepts an already-open ordinary file, supplies no authorization,
and cannot construct the private protected-launch token. This lets local tests
execute owned ELF fixtures without an authentication override. Protected policy
has no caller-selected image path, UID, numeric cap, kernel root or clock.
Opaque launch tokens and metadata cannot be deserialized as authority.

## Configuration and lifetime

Keep the existing four trusted ConfigurationPaths as an opaque typed selection.
Admit canonical UTF-8 absolute names, at most 512 bytes/16 components each, no
control characters, empty/dot/dot-dot components or duplicate role paths. Never
infer a missing role as empty. Selection establishes names, not protected contents;
the genuine worker acquires/revalidates all four files and applies frozen typed
rules under its existing closed configuration profile.

Launch tokens have one original four-second preparation window. Copy/validation
and start must finish inside it. Spawn only fixed guardian/worker roles, clear
inherited environment, and forward only selected paths and the sealed digest.
The monitor holds the image lease through descendant cleanup even if the ticket
is dropped. Its existing request deadline is also capped by the original launch
deadline, so another phase never renews preparation age.

Exec references the held memfd in the live parent's proc/fd view. The guardian
opens its own `/proc/self/exe`, requires the expected complete seal set and digest,
and holds that image for worker exec. This avoids depending on the dead manager's
PID/fd path if parent death races worker startup. The worker verifies its own
sealed image, bootstraps the real parent/session/group/NOFILE checks, then calls
the genuine collector with the typed selection. Root and namespace policy remain
in the existing adapters.

## Probe and local evidence

Add `limeos-combined-read-probe` under tests/support, behind the existing opt-in
read-worker-probe feature. Default contract mode has no host I/O. Probe mode uses
only the fixed protected image and four bounded trusted environment path values.
Guardian/worker modes require sealed current-image verification before work;
calling a role directly from disk cannot bypass bootstrap. Emit one bounded final
event only after supervised completion/refusal, with image/selection metadata and
borrowed report JSON where successful. No UID, endpoint, proc/sys root or clock
selector is accepted. Keep the existing standalone probe's combined case blocked
until external qualification explicitly adopts this separate supplied seam.

Local tests use owned ELF copies and descriptors, including a stripped copy of
the opt-in test binary (binutils; debug information stays on disk). They prove
seals, immutable bytes after source mutation/replacement, lease ownership, empty
launch environment, role/image verification, independent supervision, and genuine
nonroot collector refusal. Private protected-walk fixtures authenticate only an
owned scratch root/UID; they cannot qualify the public fixed root path. Root
positives, Docker, block probes, Pi, SSH and guest launches remain excluded.

Keep service caps intact. Memfd pages add up to 8 MiB per pinned image to the
whole process tree; no peak RSS/cgroup headroom is proved. Installed memfd/noexec,
proc/fd, loader/library trust and syscall-filter compatibility remain open. The
ELF snapshot pins executable bytes, not its dynamic loader/libraries. Source
preparation and exec can still block in kernel work; no termination/latency claim
for those paths is made. P04, defect, performance and cutover gates stay open.
Reserved backup and external qualification paths are untouched. Preserve exact
source hashes/lossless logs; builds/general scratch stay on disk, with only small
precompiled storage fixtures using owned cleaned tmpfs runtime scratch.
