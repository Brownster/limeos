# Before-image capture preflight handoff

Stream A assistance on `engineer/p04-backup-capture-preflight`, based on main
`7df3f75`. Tested product source:
`bf9a90959924617b36fe37022f22dde8ef7dad00`.
The [design](plans/2026-10-17-backup-capture-preflight-design.md)
addresses A4's missing read-only phase before private capture. The
[evidence packet](rewrite-evidence/p04/2026-10-18-backup-capture-preflight/README.md)
records exact source and local checks. This enables central whole-request
admission; it does not implement or qualify that reservation, encrypted creation,
installed backup/restore, A4/A5 or P04 completion.

## API and compatibility

`before_images::plan(sources, root, limits, cancelled)` returns an opaque
`CapturePlan` borrowing the original handles and root. It reuses the existing
selection, labels, source/root protection, type, metadata, attribute and logical
limit checks, then revalidates the entire bound set. Planning creates no private
name, copies no file content, clones no descriptor and changes no source offset.
Dropping a declined plan frees only its in-memory metadata; it performs no
filesystem cleanup or orphan adoption.

`CapturePlan::requirements()` returns component costs:

| Field | Meaning |
| --- | --- |
| objects/files/directories | Exact admitted selected object counts; directories do not select children |
| logical_file_bytes/largest_file_bytes | Exact admitted file sizes from the original metadata bindings |
| retained_descriptors | N+2 additional descriptors retained by the resulting before-image owner |
| transient_descriptors | One extra during reopen/listing; this is a code-derived component allowance |
| streaming_buffer_bytes | 65,536-byte data streaming buffer |
| bound_metadata_capacity_bytes | Bound vector and retained attribute capacities; excludes allocator overhead and other bookkeeping |
| record_byte_limit | Existing configured canonical-record output ceiling, not a promise that a record will fit |

These figures exclude already held source/parent/root descriptors, custody,
catalog, prepared replacements, writer/snapshot handles, transport and service
baseline. Logical bytes do not reserve physical blocks, filesystem metadata or
free space. Capacity/buffer figures are not a total heap/RSS bound. The scalar
summary itself is ordinary data; only the opaque plan retains actual bindings.
Neither is approval, a physical claim, a quota or a reservation.

`CapturePlan::revalidate(cancelled)` rechecks the original metadata/name/attribute
bindings and root protection without creating anything. It does not hash file
content or establish writer exclusion/consistency. `capture_planned(plan,
cancelled)` consumes that exact plan and revalidates the complete set before
attempt creation. Changed sources refuse instead of being rebound to a fresh
selection. The original root descriptor remains authoritative for private work;
replacing its pathname cannot redirect capture into a new directory.

After creation, the existing copy/hash/recheck/seal/fsync/cleanup path remains in
force. Old `capture(sources, root, limits, cancelled)` composes the new phases for
compatibility; it still supplies only per-attempt ceilings. CaptureLimits and the
version-one canonical record/identity encoding are unchanged. No owner, record or
plan can reconstruct a CapturePlan through JSON. A crash before capture leaves
no new private objects; a crash after private creation retains the previous
inert-orphan/no-adoption semantics.

## Central admission and runtime handoff

1. Keep source collection under the existing protected worker's own admission
   before opening its bounded inventory/handles. Derive the complete authorized
   managed target selection, snapshot/quiesce requirements and physical/shared
   claims centrally; do not construct owners from saved summaries or path labels.
2. Build `CapturePlan` from those still-held sources and the protected root. Compose
   its costs with the *simultaneously retained* custody, whole staged catalog,
   replacement plan/owner, original source/parent handles, service baseline and
   transients. Obtain or validate the complete reservation before capture. The
   current 256-descriptor ceiling cannot fit the 256-object owner alone (258
   retained plus one transient). Refuse or use a separately reviewed bounded
   design; do not silently raise a service limit or split an operation in a way
   that loses complete source consistency/recovery.
3. Retain current grants, shared claims/locks, original source lifetime and the
   actual reservation while consuming the plan. A requirement count or a free-
   space observation is not a reservation. Metadata preflight does not freeze
   bytes; genuine writer exclusion and online SQLite/service snapshots remain
   mandatory. Existing copy-time/full-set hash checks still verify captured data.
4. Publish durable before-image metadata and receipts only through the accepted
   central journal/recovery path after complete verification/fsync. This module
   adds no Store table, migration, protocol, job transition, approval, receipt,
   worker or activation. The root crate exports/manifests and all shared sources
   remain unchanged. Only A-owned before-image source/tests are modified.
5. Keep encryption/key recovery, complete backup inventory/media exclusions,
   actual archive creation/publication, loaded-service application/resume and
   installed crash/rollback acceptance in their A4/A5 handoffs. This preflight is
   not a substitute for those requirements. The qualification owner accepts the
   complete installed coordinator once central admission/runtime is registered.

## Local verification

The full archive crate passes **148 checks**: 143 unit/integration tests and five
compile-fail doctests, including 12 new preflight unit regressions and the new
non-deserializable-plan doctest. The existing 18-case before-image baseline
passed before implementation. Final tests include existing malicious archive,
custody/re-admission, staging, preparation, before-image and scratch-process crash
coverage; these are local library checks, not installed worker evidence. Strict
all-target Clippy and Rustfmt pass. Root Cargo.lock/manifests are unchanged.

The isolated 256-object process fixture confirms planning adds no descriptors;
the resulting owner retains exactly the reported 258. Its logged record length
is 152,862 bytes for 2,048 logical file bytes. These are fixture observations,
not universal memory/disk bounds; the transient allowance is code-derived.
Source replacement, growth, mode/parent/last-object changes and cancellation
refuse with zero attempt-creation checkpoints and no cleanup requirement. Tests
also preserve offsets, record bytes/digest, original root ownership across path
replacement and existing orphan names. No production host was contacted.

The first offline baseline could not resolve the workspace's locked Rustls
0.23.46 index entry. An authorized exact-lock fetch enabled the baseline and
subsequent offline checks; no dependency version changed. No new hosted CI,
package/VM/ARM64/Pi, whole-worker FD/RSS/task/cgroup, physical reservation,
encrypted backup, live restore, installed recovery, defect closure or cutover
acceptance is claimed.
