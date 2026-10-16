# Local read-only image-content evidence

15 October 2026. Exact tested content source:
`f0ea172be9b917b972cc57c1bbaab4ae9fae9456`, based on container-update `d8f94de`.
The separate root-pin correction is `869c74d`. Branch:
`engineer/p04-image-content-admission`.

Final checks pass: **60 Rust cases** (23 image adapter, 25 update policy, 12 B2
domain), strict Clippy (`--all-targets`, `-D warnings`), Rustfmt and Python harness
lint/format. This is one narrow independent two-crate harness, not the production
workspace/package suite. B2 API/reader/browser/UI tests were not rerun or newly
qualified. Update settings remain v1; the unregistered update report/source shape
is now v2 to distinguish root, selected manifest and config digests.

Ten synthetic OCI/Docker documents have independent Python hashlib hashes/sizes
in `tests/fixtures/deployment/images/hashes.json`. Rust verifies every exact raw
fixture. The fixtures exercise both platforms, direct/index chains and ignored
attestation entries. Layer descriptor hashes are placeholders: no layer bytes,
download, decompression, runtime contents or image health are qualified.

Tests cover raw-byte tampering, sizes/media types, wrong/missing/ambiguous platform,
unsupported qualifiers/content, consumed duplicate fields, canonical digests,
bounded sequences/documents/aggregate bytes, size overflow, DiffID relationships,
payload privacy and admitted facts entering the v2 policy without authority.
Root-pin and plain-tar/DiffID regressions each failed before their corrections.

`metadata.json` binds committed source/fixtures against Git blobs, reference
identities, tool versions, exact commands/environment, logs and limits. `harness/`
retains both the final workspace and the earlier root-regression wrapper/locks.
`make-fixtures.py.gz` preserves the exact generator run from
`.cache/image-content-leaf/make-fixtures.py`; restore it there to regenerate the
synthetic fixtures. No reference Python was imported/executed.

Retained diagnostics/final logs use lossless deterministic gzip. The earlier
59-case source and red experiments were not frozen commits and confer no final
acceptance. One Clippy invocation was queued before the test process exited;
Cargo serialized compilation with its build lock, both completed, and the final
Clippy invocation ran after both exited. Its retained log passes. No process was
killed. Shared-cache lock waits are diagnostic, not a performance measurement.
`SHA256SUMS` binds the packet.

Reproduce using `uv run --offline tests/fixtures/deployment/images/check-leaves.py`,
restore `harness/final/Cargo.lock` to `.cache/image-content-leaf/Cargo.lock`, then
run the recorded final checks sequentially with one Cargo build job. Adapt the
retained absolute wrapper/manifest paths to the reproduced checkout. Scratch,
cache and build output stay on disk under the repository. Compare source hashes
before crediting another run.

These x86_64 tests prove the supported metadata/byte-consistency subset only.
Expected root digests remain untrusted until genuine source/origin admission.
Registry/Engine collection, whole-worker budgets, signatures, complete layers,
full config validity, API/UI, execution/recurring authority, health/recovery,
ARM64/Pi/package/performance and cutover acceptance remain open. Shared roots,
production manifests/lockfile, Store/schema/protocol and A files are unchanged.
See the [handoff](../../../p04-image-content-handoff.md).
