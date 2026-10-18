# Samba document edit planning

Stream A A3 continuation from main
`7df3f7530865c31b7faefd1f7b12a25aa7655860`. Mount qualification and central
runtime files remain claimed elsewhere. Implement the missing owned-section
replacement in A-owned `shares.rs` and a child module, without changing shared
roots, imports, routes, workers, Store, journals, effects or activation.

## Existing behavior and choice

The Rust renderer already emits `# BEGIN LIMEOS SAMBA V1` / end markers around
checked typed sections. The frozen Python plugin at `80593b2` generates sections
for manual copying into smb.conf; it does not provide a safe whole-file editor.
A3 requires preserving operator sections during atomic managed application.

Add a pure document-edit planner over supplied source bytes, optional trusted
previous typed configuration and desired typed configuration. Reusing a generic
INI serializer would rewrite operator formatting and interpret unsupported
directives. Replacing any marker-delimited text would treat comments as ownership
and overwrite manual edits. Instead, accept an existing block only when it exactly
matches the previous checked renderer; splice only that block. Without previous
ownership, append a new block only if no existing marker or colliding section
claims the names. No unmarked legacy section is silently adopted.

## Invariants

Preserve every operator byte outside the selected span, including comments,
case, spaces, UTF-8 and line endings. Existing canonical LF/CRLF blocks and a
missing final newline retain their style. Appending adds a necessary line break
without rewriting the preceding bytes. Empty/disabled desired shares remove
owned sections only; no user, directory, account or service is changed.

Scan bounded logical lines for external section collisions using conservative
case/whitespace folding. Handle continued headers/parameters; a marker within
a continuation refuses. Duplicate, nested, reversed, unsupported-version or
modified markers/owned bodies refuse. Replacement also refuses a suffix whose
first semantic line is a parameter: otherwise changing the final managed share
would change which section owns those preserved bytes. Unknown operator
directives remain byte-preserved; they are not imported or semantically approved.

All source and candidate documents are bounded to 256 KiB, logical lines to
4 KiB and external section count to 1,024. Invalid UTF-8/NUL/control text,
unfinished continuations and ambiguous syntax refuse the whole proposal with
fixed finding paths, never echoed source text. Larger documents remain explicit
unsupported input, not a migration retirement. Existing share validation/bounds
stay unchanged. Reserve candidate space fallibly before copying it.

An opaque nonserializable plan borrows source/desired configuration and owns
only the candidate. `check_current` compares exact source bytes; it supplies no
path, descriptor, inode, namespace or effect authority. Debug and the optional
serialize-only summary reveal counts only. Runtime must retain protected source
owners, complete include/registry/global semantics and claims, then run fixed
testparm, apply atomically under the common journal/locks, independently verify
reload/share state and recover failures. Text markers alone never authorize edits.

The [Samba manual](https://www.samba.org/samba/docs/current/man-html/smb.conf.5)
describes section/parameter syntax, name comparison, continuations and literal
includes. This editor does not resolve include files, registry or usershares,
validate global inheritance or declare a share safe/available. A full Samba
parser/importer or passing text checks cannot replace actual runtime validation.

## Validation

Use the existing frozen Samba JSON and generated-section fixtures, plus document
cases with surrounding operator sections, mixed/literal paths, LF/CRLF/EOF,
removal, stale bytes, collisions, continued logical lines and malformed ownership.
Verify byte equality outside the splice and no candidate on refusal. Compile-fail
checks cover JSON construction and source lifetime. Run baseline share cases,
full domain tests, strict Clippy and changed-path formatting sequentially with
one Cargo job and repository disk scratch. Execute no Samba process or host
write. Bind source/reference hashes and preserved failures in the handoff; A3,
P04 and installed recovery/resource/cutover gates remain open.
