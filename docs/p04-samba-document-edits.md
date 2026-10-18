# Samba document edit handoff

Stream A A3 input on `engineer/p04-samba-document-edits`, based on main
`7df3f7530865c31b7faefd1f7b12a25aa7655860`. Tested product source:
`67e127b4afdefa87faa022b7940a7a949ad70039`.
See the [design](plans/2026-10-18-samba-document-edit-design.md) and
[local evidence](rewrite-evidence/p04/2026-10-18-samba-document-edits/README.md).
The existing checked share renderer remains unchanged. This adds document
composition needed before its eventual atomic runtime application.

## Supplied API

`limeos_domain::shares::document::plan_document_edit(source, previous, desired)`
returns an opaque `SambaDocumentEdit` borrowing the exact source bytes and desired
typed configuration. `candidate()` exposes candidate bytes; `desired()` exposes
the borrowed policy. `summary()` serializes only change/replace flags, byte
counts and enabled-share count. Debug contains the same counts, without source,
share paths, credentials or operator directives. JSON cannot create a plan and
the plan cannot outlive its source. No file, process or service is accessed.

`previous=None` appends the generated block only when there is no existing
LimeOS marker or colliding external section. It adds a line break if needed,
preserving every original byte. An existing block requires `Some(previous)`
and an exact match to that typed configuration's renderer. Changed directives,
comments or spelling inside the block refuse; they are not overwritten. Missing
expected blocks, foreign blocks, malformed/duplicate/nested/reversed markers
and unknown marker versions refuse. An unmarked frozen Python section is never
silently adopted merely because its name or contents match.

Replacement splices only the owned span. Operator bytes before/after it retain
their original case, whitespace, comments, values and line endings. Canonical
LF/CRLF blocks and an EOF without a final newline retain that style. Empty or
disabled desired configuration removes owned share sections while retaining the
markers and operator content. Reapplying unchanged desired configuration yields
an identical candidate and `changed=false`; this is no service-health proof.

Outside the owned span, the planner scans bounded logical lines for potential
case/whitespace collisions with both desired and previous names, including
disabled or removed names. Continued headers cannot hide a collision. A suffix
must start with a section header before any semantic parameter; otherwise an
edit could move preserved directives onto a different final managed share.
Markers within a continuation refuse. Non-ASCII external section names and
ambiguous standalone comment continuations remain explicit unsupported inputs
until qualified Samba name/parser behavior is available. UTF-8 operator values,
comments and literal checked share paths remain byte-preserved.

Bounds are 256 KiB for source and candidate independently, 4 KiB for admitted
logical-line input and 1,024 external section headers. Invalid UTF-8, NUL and
non-text controls refuse. Findings contain fixed field paths/codes rather than
source text. Candidate capacity is reserved fallibly before copying; marker
recognition compares a fixed prefix without allocating a lowercased document
line. These component bounds are not whole-worker RSS or reservation evidence.

`check_current(current)` requires exact equality with the original source bytes.
It does not establish retained inode/name identity, mount namespace, permission,
physical backing, trusted ownership history or current authorization. The plan
factory accepts supplied bytes; opaque/nonserializable does not make that input
a protected file owner or grant permission to publish it.

## Central runtime requirements

Retain a genuine protected complete Samba configuration selection and the trusted
previous managed ownership record under current authority and all shared claims.
Source markers or caller-supplied previous JSON alone never establish ownership.
First adoption of an unmarked legacy section needs a separately reviewed import
and migration explanation; this editor intentionally cannot take it over.

The [Samba manual](https://www.samba.org/samba/docs/current/man-html/smb.conf.5)
describes include and registry configuration alongside section syntax. This
editor preserves opaque external directives but resolves none of them. A
candidate can still have include/registry/usershare collisions or unsafe global
inheritance. Before execution, the registered runtime must resolve the complete
configuration and independently validate the effective result with fixed
`testparm`, then refuse unsupported or changed inputs. No semantic acceptance
or executable readiness is implied by successful document composition.

The integrator still owns approved operation/protocol registration, root ceilings,
complete physical/pool/share/stack claims, shared locks and durable recovery.
Use independent before-images and verified descriptor-relative atomic publication,
with current file/name/metadata checks and fsync. Reload only after validation;
independently verify service/share state and record a verified return/rollback
on failure. Missing disks, dependent writers, accounts/permissions, cancellation,
helper death and uncertain outcomes retain their separate runtime requirements.

The existing qualification owner must accept installed testparm/apply/reload,
operator-section preservation, failures and real restart/recovery. Direct
`smb.conf` semantic import remains open, as do complete A3, P04 and cutover.
No UI, route, read query, Store/schema, protocol, package or service was changed.

## Local checks

Final actual domain crate: 164 unit tests plus five compile-fail doctests pass,
including 21 new document cases and two new lifetime/construction doctests.
The pre-change share filter passed 14 cases with 129 filtered. Strict all-target
Clippy, Rustfmt and whitespace checks pass. The first full suite also passed;
the final run follows the fixed-prefix allocation improvement. Tests use the
existing frozen Samba fixture plus synthetic supplied documents, with no Samba
binary, host file, Pi or installed qualification run. Input/artifact and frozen
reference hashes are in the packet. The historical storage inventory latency
gate remains failed; no resource/performance, defect or milestone closes here.
