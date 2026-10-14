# Retained protected Compose files

`limeos_executor_container::compose_files::inspect_compose_files` adds an
unregistered read-only prerequisite for actual stack-file binding. It takes a
checked names-only `ComposeFileSelection` and a cancellation callback. No route,
RPC, executor command, service or package activation invokes it.

Select one canonical absolute working directory, one existing `ComposeFileName`
variant and whether to include the fixed `limeos.override.json`. The recognized
operator names are `compose.yaml`, `compose.yml`, `docker-compose.yaml` and
`docker-compose.yml`. Only the selected operator basename may exist. An alternate
regular file, directory, FIFO or dangling symlink refuses the whole request,
without reading its contents. The operator file is mandatory; the managed file
must exist exactly when explicitly selected. Custom overrides, empty stack
discovery and other file-selection conventions need separate support.

Every derived path is at most 512 UTF-8 bytes and 16 components. Working directories
have at most 15 components, each at most 255 bytes. Empty/dot/dot-dot components,
controls, commas, repeated/trailing separators and relative paths refuse. Literal
case, spaces and Unicode are preserved. The selection constructor validates names
before allocation; it grants no catalog authorization or management identity.

Public collection requires actual effective UID 0 before namespace or target
access. Fixed `/proc/self/ns/mnt` and `/proc/1/ns/mnt` references must be on nsfs
with equal device/inode identities before and after the root open. The owner
retains a namespace reference. The Linux reference mechanism follows pinned
[kernel nsfs source](https://github.com/torvalds/linux/blob/v6.12/fs/nsfs.c);
installed syscall/confinement compatibility is unqualified. This establishes a
PID-1-relative mount view, not independent physical-host, user/PID namespace or
Docker-daemon identity.

Walk from the retained root with descriptor-relative no-follow opens. Root and
every ancestor must belong to UID 0 without group/world write permission. Retain
all traversed directories and both selected read-only files. Classify leaves
with O_PATH before opening a regular, protected, single-link file for nonblocking
content reads. Compare classification and read descriptors before reading. A
symlink, FIFO, device, directory, hardlink, unsafe mode or wrong owner refuses.
No special leaf is opened for content.

`ComposeFileEvidence::snapshot()` reports the original issue time, selection,
mount view, root and ancestors, ordered operator/managed file paths, full file
identities, raw SHA256 digests and absent fixed basenames. Object identities keep
device, inode, mount ID, mode and UID/GID. Files also keep link count, size and
mtime/ctime. Directory timestamps are outside identity so unrelated entries may
change. Raw bodies, environment and credentials are absent from reports. Reports,
selections and owners cannot deserialize; metadata cannot reconstruct descriptors.

Hash arbitrary raw bytes through a fixed 4 KiB stack buffer, using positional
reads and a growth sentinel between full file-identity checks. Each file is at
most 64 KiB, with one operator and at most one managed file. Empty/binary contents
can supply file facts; they do not establish valid Compose semantics. No full
body is retained, YAML is never parsed or reformatted and no file is written.
The existing workspace SHA256/hex pins become direct container dependencies,
without new versions or a registry download.

Collection revalidates before returning. Each subsequent `revalidate` reacquires
the protected root/mount view, freshly walks ancestors and compares fresh and
held file identities/hashes with the original observations. It checks fixed
basename presence/absence before and after reads, then rewalks ancestry and checks
selected named objects again. Late file or directory replacement, changed content,
metadata or absence refuses. Separate files and comparisons are observations,
not a writer transaction against privileged changes.

The original monotonic/wall four-second lifetime never renews. Each validation
pass has a two-second cooperative deadline and cancellation checks between reads
and traversal steps. Every failure, cancellation, reentrant/concurrent refusal
or unwind permanently invalidates that owner. A first check cannot restore
readiness after another check refuses. Borrowed reports remain past facts after
invalidation or expiry. Synchronous kernel work still needs independent supervision.

A fixed shared 1 MiB lifetime ledger charges requested read bytes before each
syscall, including repeated fresh/held comparisons and sentinels. It never refunds
or replenishes requests; short reads can exhaust it conservatively. At most 19
descriptors are retained, including the namespace reference, with a conservative
six additional transient descriptors during validation. A counting writer bounds
the complete compact report to 16 KiB without allocating a second report buffer.
These limits qualify no combined-worker peak memory/RSS/cgroup headroom. The
existing combined resource model and worker/probe remain unchanged.

[Local acceptance](rewrite-evidence/p04/2026-10-14-compose-files/README.md) covers
24 owned-descriptor cases and three compile-fail cases. Successful file fixtures
use explicitly private scratch-root/UID/clock substitutions through the genuine
walk/hash code. A primitive test checks real nsfs references; public nonroot
refusal occurs before the callback or namespace/target access. These tests qualify
no public root-positive, foreign-namespace or installed collection.
See the [design](plans/2026-10-14-compose-file-evidence-design.md).

The [Engine membership owner](p04-compose-membership-evidence.md) still describes
declarations separately. This slice does not bind its labels or a P03 approved
catalog to actual files, infer a stack resource key, read `.env`/includes, evaluate
interpolation or combined Compose semantics, or establish complete consumers.
Approved catalog/hash/management binding, genuine Engine composition, claims,
locks, approvals, effects/recovery, independent review, worker integration and
installed qualification remain open. The signed ARM64 candidate excludes this
slice. No Pi, SSH, real Docker, raw block probe, guest, reserved recovery path,
capability, service ceiling or cutover changed.
