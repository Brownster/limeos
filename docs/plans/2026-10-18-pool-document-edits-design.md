# MergerFS fstab document proposals

Stream A leaf based on assistance head `6eb3b37`. Add `pools::document` inside
the already owned domain module. Runtime balance still needs an accepted FUSE
identity/claims contract; this independently prepares the required A3 managed
configuration edit without inventing that authority.

Replace only an exact previous typed `# BEGIN LIMEOS MERGERFS V1` block. Preserve
all external bytes, comments, ordering, CRLF/LF and EOF style. Append only when no
owned block or target collision exists; never adopt an unmarked/legacy block.
Require prior typed configuration if a marked block exists, including an empty
one. Disabled/removed configurations still reserve their old target names during
collision checking. Keep an empty owned block when all pools are disabled.

Use a closed collision scan, not a complete fstab semantic importer. Admit six
fields, numeric dump/pass fields, opaque options and trailing comments. Decode
three-digit octal escapes only for target comparison, preserving original bytes.
Refuse malformed/ambiguous target syntax, continuations, NUL/control bytes,
noncanonical target components and foreign targets equal to or below any old/new
pool target. Other parent mounts and literal case remain untouched. Complete
source/options/alias/runtime semantics remain the protected reader's obligation.

Match the existing reader's 64 KiB document and 128-entry ceilings; bound physical
lines to 4 KiB and decoded targets to 512 bytes. Share the existing canonical
renderer through a private fmt writer so the proposal can bound its output while
the public renderer preserves every frozen output byte. Add no new options or
boot-time behavior. An opaque borrowed edit exposes candidate bytes and a
count-only summary, with exact current-byte recheck. It cannot be deserialized or
outlive its original bytes/configuration.

Fixtures must cover existing layouts, unrelated storage/operator rows, escaped
and nested collisions, stale/duplicate/legacy markers, old/disabled targets,
styles, empty removal and every bound. Run native domain tests, strict all-target
Clippy and formatting with one Cargo job/disk caches. No host fstab, mount,
systemd, API/UI, schema, registry, lock, helper or activation changes. The
integrator retains protected ownership, full claims, atomic publication,
before-images, reload/verification and durable recovery acceptance.
