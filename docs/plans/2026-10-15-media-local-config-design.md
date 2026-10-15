# Local media download configuration drafts

Stream B follow-up to `25c9cbe`. Port the frozen Transmission and SABnzbd local
directory seeding rules as pure supplied-byte planners. No filesystem read,
credential lookup, process, HTTP call or write is involved. A missing source is
unavailable, distinct from an existing empty SABnzbd configuration.

Use explicit Transmission legacy kebab-case and SABnzbd dialects. Desired
complete/incomplete paths are literal absolute container paths, with bounded
UTF-8 text and no equal/ancestor overlap. Deriving them from the actual storage
contract, resolved binds and service identity remains an integration prerequisite.
When no incomplete path is requested, preserve existing incomplete settings,
matching the frozen rule rather than disabling them.

Transmission changes only `download-dir`, `incomplete-dir` and
`incomplete-dir-enabled`. Validate a bounded JSON object, refuse duplicate root
keys and conflicting modern selected-key aliases, then replace selected value
spans or insert missing fields. Preserve all unrelated bytes, including numeric
spelling, whitespace and secret-bearing settings. Do not reserialize the file.
The dialect is explicit because newer Transmission versions support snake-case
settings; absence of an alias alone does not prove the installed save format.

SABnzbd changes only `complete_dir`/`download_dir` in the top-level `[misc]`
section. Preserve preamble, comments, inline comments, line endings, section order
and nested server sections. Reject duplicate/ambiguous selected declarations,
malformed section syntax and unsupported multiline/quoted-section constructs.
This is a conservative targeted ConfigObj subset, not a full INI validator.
Missing keys are inserted before any nested section; missing `[misc]` is appended
as a new top-level section. Render new path values quoted.

Bound input and output to 64 KiB, paths to 512 bytes, root JSON fields to
128, INI lines to 2048 and nesting to 32. Charge generated fields/lines too, so
output remains admissible on rerun. Refuse scalar/section name collisions in
SABnzbd's root and selected section. The opaque non-deserializable draft
exposes optional artifact bytes only for a future protected writer. Debug and
serializable summaries contain no source values or artifact contents. A no-op
returns no artifact. All source/instance, physical path/ownership, quiescence,
authority/locks, before-image/recovery and service-result gates remain unresolved.

Use synthetic frozen-case fixtures with fake secret sentinels, byte-preservation
and rerun checks, malformed/duplicate/modern-format cases, CRLF/nested/preamble
regressions and input/output/depth limits. Run the existing isolated domain
harness, strict Clippy and Rustfmt sequentially; source-bind the evidence. Shared
manifests/roots, Store/schema, runtime/protocol, API/UI and A work stay untouched.

Future execution must verify the exact owned file and metadata, current container
instance/format, mounted dependencies, numeric identity, verified service stop,
central claims/locks and durable before-images. Transmission may overwrite file
edits on shutdown; never treat a draft as permission to edit a running service.
Apply atomically only under the central recovery path, then independently verify
the loaded service configuration. Installed/ARM64/Pi qualification remains with
the existing test owner; this does not complete media seeding or P04.
