# Before-image capture preflight

Stream A assistance, based on main `7df3f75`. A4 requires whole-request admission
before private capture. The existing before-image library validates selection
inside `capture`, then immediately creates its attempt. A coordinator cannot
inspect the bound source set's resource requirements before authorizing that
private work. Its 256-object case retains 258 additional descriptors, before
caller/catalog/preparation state; the 256-FD service cannot admit that request.

Add an opaque, nonserializable `before_images::CapturePlan` produced by a
read-only `plan` function from the same borrowed source handles, root and limits.
Reuse existing source/root/label/type/attribute checks. Retain those exact bindings
and expose a scalar requirements summary: object/file/directory counts, exact
logical file bytes/largest file, N+2 retained descriptors, one transient
descriptor, the 64-KiB streaming buffer and the configured record-byte ceiling.
The buffer/record figures are component costs, not total heap/RSS bounds.

Planning creates no private name, copies no bytes, clones no descriptor and
does not move source offsets. Borrowed caller handles must stay alive. The plan
is metadata/read evidence rather than a content digest, approval, physical claim,
quota or global reservation. The central worker still composes the complete
custody/catalog/preparation/source/parent/baseline/transport costs and acquires
its actual reservation. Do not add a second admission or resource registry.

`capture_planned` consumes that plan against its original root, polls cancellation
and rechecks all original source bindings and root protection before creating an
attempt. Changed names/inodes/metadata/attributes or writable descriptors refuse
instead of silently replanning. Once capture starts, retain all existing copy,
hash/recheck/seal/fsync/cleanup behavior. No saved plan can be deserialized or
adopted after restart. Writer exclusion and genuine source authority remain the
caller's responsibility; metadata preflight does not freeze content.

Keep existing `capture` compatible by composing the new phases internally.
Preserve CaptureLimits, canonical record version/bytes/digest and existing opaque
owner APIs. No lib.rs/Cargo/schema/protocol/worker/service change. A caller using
the old convenience entry point still has only its per-attempt ceilings; it gains
no whole-request reservation by using this refactor.

Tests use the existing genuine scratch-file fixtures: no planning artifacts or
offset/FD growth, exact scalar costs, decline/drop without capture, consume-once
capture equivalence, original-root binding, changed source/type/name/parent/mode,
cancellation before creation and existing fault/cleanup regressions. The complete
archive crate runs sequentially with one Cargo job and disk-backed scratch.
Installed worker admission, encryption/key recovery, actual backup runtime and
live restore/recovery remain separate unfinished A4/A5 requirements.
