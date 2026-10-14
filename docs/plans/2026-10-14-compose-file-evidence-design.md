# Retained protected Compose file observations

Base: local main `19beb67`. Engine-declared Compose names do not establish actual
files. Add an unregistered read-only prerequisite for the P03 catalog's supported
single operator file plus the separate `limeos.override.json` file.

## Selection and boundary

An explicit names-only selection chooses a canonical absolute working directory,
one of the four existing `ComposeFileName` variants and whether the fixed managed
file is included. Bound every derived file path to 512 UTF-8 bytes and 16
components, with no controls, comma, symlink, empty/dot/dot-dot component or
component above 255 bytes. Selection supplies no catalog authorization or ownership.

Public collection requires effective UID 0 before filesystem access. Compare
retained `/proc/self/ns/mnt` and `/proc/1/ns/mnt` object identities before opening
the root and afterwards. Recheck this PID-1-relative mount view on validation.
This does not independently identify the physical host or Docker daemon namespace.
Walk from the root with descriptor-relative no-follow directory opens. Root and
all traversed directories must belong to UID 0 without group/world write access.
Retain every ancestor and the selected read-only files, with device/inode/mount,
mode and ownership identities. Ignore directory timestamps so unrelated entries
can change without changing the directory object.

Probe all four recognized operator basenames without reading alternate contents.
Only the selected basename may exist; any alternate object, including a dangling
symlink or special file, refuses. The selected operator file is mandatory. The
fixed managed file must exist exactly when explicitly selected. Missing names
are absence observations, not safe-shutdown authority. Classify leaves with
O_PATH before opening a regular, protected, single-link file for nonblocking
read access; compare both descriptors before reading. Never open a FIFO/device
for content. Refuse replacement, permission, ownership, link or content changes.

## Lifetime and bounds

Hash raw bytes with the existing workspace SHA256 dependency, including arbitrary
binary data. Stream through a fixed 4 KiB stack buffer; no content or secrets enter
reports and no full file body is retained. Each selected file is at most 64 KiB,
with at most two files. Positional reads and a growth sentinel are bracketed by
full file identities, including size and mtime/ctime. Collection and revalidation
compare held and freshly walked names/content, then recheck absence and ancestry.
They observe changes; separate files and hashes are not a writer transaction.

Keep a fixed original four-second monotonic/wall lifetime and a two-second
cooperative work deadline per pass. A shared 1 MiB lifetime ledger charges requested
read bytes before each syscall, including repeated comparisons and sentinels;
successful validation never replenishes it. Count at most 18 retained descriptors,
with a conservative six additional transient descriptors during validation.
Count the complete compact metadata report against 16 KiB without allocating a
second report. These bounds qualify no combined-worker peak allocation/RSS.

Use an atomic owner-local validation guard. Failure, cancellation, reentrancy or
unwinding permanently invalidates the owner. Successful checks preserve its
original lease, bytes ledger and observations; they cannot restore a concurrent
refusal. Reports and owners cannot deserialize. Names-only selection is also
serialize-only, with a checked explicit constructor.

## Local acceptance and remaining work

Test genuine scratch descriptors through the production walk/hash code with an
explicitly private root/UID/clock seam. Cover all four basenames, optional managed
files, ambiguity and changed absence, arbitrary bytes and secret omission,
symlink/hardlink/FIFO/directory leaves and ancestors, modes/ownership/replacements,
content and metadata changes, held/fresh disagreement, unchanged unrelated entries,
original age, cancellation, reentrancy/unwinding, exact path/file/read/report/FD
bounds, non-deserialization and public nonroot refusal before target access.

Keep lossless diagnostic and accepted logs, exact-source/source/artifact hashes,
full Rust/strict Clippy/fmt/contracts/repository and local ARM64 harness checks.
Builds and scratch stay on disk except already compiled existing storage fixtures'
small owned cleaned tmpfs runtime. No Pi, SSH, real Docker, block probe or guest.
Reserved backup and external qualification paths are untouched.

Do not parse or execute YAML, interpolate environment, read `.env`, follow includes,
select arbitrary overrides, write files or infer a managed stack from their presence.
Binding an approved catalog's digests/management identity and genuine Engine owner,
actual combined Compose semantics, claims/locks/approvals/effects/recovery,
combined-worker budgeting/integration, independent review and installed root/
namespace/confinement/performance qualification remain open. No package, route,
RPC, schema, service activation, capability or cutover changes.
