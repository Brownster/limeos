# MergerFS fstab document edit handoff

Branch `engineer/p04-pool-document-edits` starts from assistance candidate
`6eb3b37`. It adds a byte-preserving MergerFS owned-block proposal inside the
existing A-owned `pools` module. This prepares A3's protected configuration
publication path; it does not execute a mount, balance, reload or file write.
The [design](plans/2026-10-18-pool-document-edits-design.md) defines its boundary.
Tested product: `e43ac89560faadfb149d036ce5a6e636acb84cb5`.

## Supplied interface

`pools::document::plan_document_edit(source, previous, desired)` returns an opaque
borrowed `PoolFstabEdit`. It exposes candidate bytes, the original borrowed desired
configuration, a count-only summary and an exact `check_current` byte comparison.
It cannot be deserialized or outlive its source/configuration. Debug and summary
serialization contain no operator entries, options, paths or credentials. Caller
access to candidate bytes is explicit; those bytes are private configuration.

A private fmt writer now supplies both `PoolsConfig::render_fstab` and this
bounded renderer. The public renderer preserves the frozen bytes, ordering,
policies, presets, options, enablement and numeric storage identity behavior.
No new mount options, timeouts, fstab ownership marker version or schemas were
introduced. No shared crate root, Store, protocol, claims, routes, service,
package or frontend files changed.

Replace only an exact previous typed `# BEGIN LIMEOS MERGERFS V1` block. A marked
block, even empty, requires previous typed configuration. An unmarked document
requires no prior ownership claim. Neither supplied JSON nor marker presence
proves protected ownership; the registered caller must obtain that independently.
Stale, duplicated, malformed, nested, indented, case-changed or unsupported-version
markers fail. The old pi-health marker family requires explicit migration/import,
and an unmarked matching row is not silently adopted.

External bytes retain their ordering, comments, options and spelling. Existing
managed blocks retain LF/CRLF and their final-newline state; first append chooses
the last external newline style and adds only the necessary separator. Disable
or removal emits the existing renderer's empty owned block and preserves external
bytes. Old, desired, disabled and removed pool target names all participate in
collision checks. A foreign target equal to or beneath any such target refuses.
Root/parent mounts, component-prefix neighbours and literal case remain distinct.

## Closed collision profile and bounds

The scanner accepts six-field external entries, space/tab delimiters, u32
dump/pass fields and trailing comments after the six fields. Source, filesystem
and option fields remain opaque, not semantically accepted. Targets support
one-pass three-digit octal decoding for comparison, including escaped UTF-8,
space, tab and backslash, while the source bytes stay untouched. Reject unknown,
short, overflowing, NUL/control or invalid-UTF-8 target escapes, relative/quoted
targets and noncanonical slash/dot components. Apply the same target-spelling
profile to typed previous/desired targets, including disabled ones. The public
configuration validator/renderer remains unchanged; unsupported document cases
return findings rather than normalized replacement data.

The [fstab manual](https://man7.org/linux/man-pages/man5/fstab.5.html) describes
per-line records and escaped field whitespace. The
[util-linux decoder](https://github.com/util-linux/util-linux/blob/master/lib/mangle.c)
provides the upstream octal/one-pass reference. This editor intentionally does
not implement the full libmount/systemd syntax or their differing semantics.
Valid shorter fstab records, quoting, continuations and unsupported target forms
require another reviewed profile/import path, rather than silent reinterpretation.

| Component | Bound |
| --- | --- |
| Original and final candidate | 65,536 bytes each |
| Physical line | 4,096 content bytes; writer checks cumulatively across fmt pieces |
| Entries | 128 total, independently checked with previous and desired enabled rows |
| Decoded external/owned target | 512 bytes |
| Typed configuration preflight | Existing maximum 32 pools, 32 branches/pool, 64-byte id/name, 512-byte paths, 32-byte size strings; checked before relationship/findings work |
| Managed render buffer | 65,538-byte reservation; permits trimming CRLF at EOF, then enforces the 65,536-byte block bound |

These are component bounds, not measured peak RSS, latency or whole-worker
admission. New document proposals can refuse a configuration the broader public
renderer accepts, for example a rendered line over 4 KiB or a quoted target.
The current central fstab reader's profile still rejects CRLF; this pure editor's
preservation support does not widen that reader or grant installed CRLF parity.
Unsupported input must remain visible until its actual read/semantic profile is
accepted.

## Runtime and qualification handoff

Consume genuine retained configuration/file owners and the trusted previous
managed ownership record. Recheck exact source bytes plus original named file,
metadata, physical/namespace identities and current policy. A matching byte buffer
does not prove any of those identities or authorize publication. Re-render from
typed intent under the approved operation, complete old/new device/pool/stack/
share/protection/backup claims and current grants/approval. Resolve the complete
configuration, source aliases and active dependencies before accepting effects.
Opaque external rows and successful text composition do not prove safe absence.

The integrator retains operation/protocol/route registration, ceilings, common
locks, before-images and durable journal transitions. Publish descriptor-relative
and atomically with fsync, verify actual manager/mount state, retain uncertainty
barriers and provide verified rollback/return. B's verified dependent-stop/backup
coordination contract remains required. This leaf does not supply A2 bounded boot
waits, runtime mount-loss handling, FUSE admission, MergerFS balance supervision
or MFS-001's real exit/timeout/missing-binary proof.

The existing qualification owner still needs installed actual fstab validation,
apply/reload/mount, operator-entry preservation, missing disk, dependent writers,
interruption before/after each publication/receipt boundary, restart and return
path acceptance. No host/Pi fstab, systemd, helper, package or network operation
ran here. A1/A2/A3/A3b/A4/A5, P04, defects, performance and cutover gates remain
separate and open.

## Local evidence

The [source-bound packet](rewrite-evidence/p04/2026-10-18-pool-document-edits/README.md)
records the final tested product identity, inputs, actual logs and exact checks.
Final domain results are 207 unit tests plus seven compile-fail doctests, including
29 new unit cases and two borrowing/construction doctests. Strict all-target
Clippy and changed-file formatting pass. These checks use the existing frozen
layouts and synthetic supplied documents only. No frontend, broad package,
installed, hardware, resource or performance suite is credited.

The baseline `pools` filter passed eight cases with 170 filtered. Initial full
checks passed, then an additional shape preflight was verified. Final review
added a regression showing quoted typed targets bypassed the external spelling
guard; the retained red run reproduces it. The fix reuses the same target guard
for owned configuration. Repeated cases are not added to the final total.
Inherited assistance packets retain their original source identities and need
their original checkouts; this new leaf has its own input manifest.
