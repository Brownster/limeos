# Private managed-target before-images

Base `1401c255d4a05b8621c2dcd497d6f7f0cbe91d56`; branch
`engineer/backup-before-images`; worktree `/tmp/limeos-backup-before-images`.
The operator authorized this next slice after the rewrite status review.
No applicable AGENTS.md exists in the repository or parent hierarchy.

Estimate: **24 human engineering hours**, with scope review at **36 hours**.
P04 keeps its 320-hour estimate and 480-hour review threshold. Cumulative human
effort is unmeasured; wall time, agent activity and tool waits are separate.

## Contract

Add an independent `before_images` archive-library module. The caller supplies
each existing managed object as a borrowed read-only file descriptor, retained
read-only parent descriptor and single-component current name, together with
trusted resource/relative-path labels. No managed pathname is opened by the
library. Current name/inode, ordinary type, metadata and complete content are
checked before and after copying, then checked across the whole selection
before return. Source descriptors and their offsets remain caller-owned.

Capture existing regular files and directory metadata only. Directories do not
recursively select children. Refuse hard-linked files, special permission bits,
extended attributes other than `security.selinux` (including ACLs/capabilities), writable/O_PATH descriptors,
unsupported types and invalid/duplicate/colliding labels. Record numeric
UID/GID, ordinary mode, modification/change times, device/inode evidence and
the exact optional SELinux label bytes (bounded to 4096 bytes). The local host
applies that label to every scratch object; the first test run established that
an unconditional attribute refusal admitted no fixture. This explicit supported
attribute does not authorize applying or rewriting a live security label.
Access times, birth times, inode flags, sparse layout, absent targets, database
consistency and a complete directory-tree snapshot are outside this contract.
Reads may update source access times. Failures querying required metadata
refuse; unsupported metadata is never silently presented as preserved.

The caller authorizes the descriptors and labels, proves ancestor/mount/backing
and workspace separation, excludes all writers and coordinates services. Name
and metadata rechecks detect observed changes but cannot replace those locks or
prove a simultaneous multi-file snapshot. The module supplies no root policy,
quiescing, SQLite online snapshot, quota reservation or execution authority.

An existing effective-UID-owned private root must have exact mode 0700 and only
the optional bounded SELinux attribute. Open it with openat2 NO_SYMLINKS/NO_MAGICLINKS. Retain its
descriptor and create an exclusive random `before-images-<hex>` attempt. Use
flat generated names, never source labels. Independently copy/hash regular
files, create empty private directory objects for metadata entries, seal files
0400/directories 0500, reopen read-only, fsync each object/attempt/root and
verify the complete private set. A bounded canonical in-memory record binds
labels, source facts and checksums, excluding random private inode/name facts.

Return an opaque ephemeral `BeforeImages` owner, borrowed checked readers,
metadata, record identity, whole-owner revalidation and explicit discard.
Serialization cannot reconstruct an owner. No durable marker/adoption API or
live replacement exists. SIGKILL leaves an inert orphan; fresh capture creates
a new attempt and leaves that orphan alone.

Preserve primary errors plus observable cleanup ambiguity. Cleanup checks
owned names/inodes, retains foreign/unverified objects and attempts once; Drop
does not retry an explicitly failed cleanup. Streaming buffers, counts,
aggregate bytes and canonical record growth have finite caller-supplied limits
and checked arithmetic. Limits have no production defaults.

## Ownership and validation

Own new `crates/backup-archive/src/before_images.rs`, its private helpers/tests,
new integration tests, this design, API documentation and dedicated evidence.
The only shared edit is the additive `pub mod before_images;` export. Preserve
preparation, custody, staging, active reconstruction, core/schema/contracts,
Cargo dependencies, packaging, services, UI and the frozen Python project.

Use real local scratch descriptors and bytes. Test source replacement,
truncation/append/mutation, unsupported metadata/type/flags, bounds, short and
interrupted I/O, corrupt private writes, fsync failures, cancellation, cleanup
ambiguity, checked-reader mutation and real owned-child SIGKILL/reaping.
Include compile-fail owner/reader constraints, focused/full Rust tests,
formatting, strict Clippy, repository/contracts and unchanged dependency checks.
Preserve raw failures and exact source identities. No Pi/SSH/wybie, workstation
Docker, installed guest or native ARM64 runtime qualification is claimed.

**BKP-001 stays open.** Durable before-image publication/receipts, protected
destination replacement, installed metadata application, shared physical
claims, independent worker admission/deadlines, service/database snapshots and
power-loss recovery remain integrator work. The historical inventory latency
failure remains unchanged.
