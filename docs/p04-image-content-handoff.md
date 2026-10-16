# Image-content admission integration handoff

Engineer B, 15 October 2026. Branch `engineer/p04-image-content-admission`,
based on container-update `d8f94de`. Root-pin correction: `869c74d`.
Final tested content source: `f0ea172be9b917b972cc57c1bbaab4ae9fae9456`.
This follows B2/container-update policy; it does not include B4 catalog or the
package/HOST-001 branch. Shared runtime and A's active mount files are untouched.

## Delivered behavior

`crates/executor-container/src/deployment/images.rs::admit_image_content` accepts
borrowed root/optional-child/config bytes, an expected SHA-256 and an expected
supported platform. It returns an opaque `AdmittedImage` containing bounded digest,
platform, layer count and size metadata. No payload/runtime settings are retained.

- Hash exact original bytes. Verify the root reference, index-to-child descriptor
  size/hash/media type and manifest-to-config size/hash/media type. A direct root
  cannot carry an ignored extra child. An index needs the selected child bytes.
- Support OCI and Docker schema-2 manifest/index/config types for linux/amd64
  base v1 and linux/arm64 base v8, including omitted base variants. Require one
  unambiguous matching index entry and matching config platform. Higher variants,
  unresolved OS requirements, nested indexes, matching unsupported types,
  artifacts and selected external/embedded content are refused.
- Nonmatching platform/attestation entries never become fallback candidates.
  Duplicate consumed fields are refused. Unknown metadata is skipped; this is
  selective content admission, not validation of every OCI/config field.
- Check canonical SHA-256 descriptors/DiffIDs, rootfs type, matching layer counts,
  supported layer media types and size-sum overflow. Compressed layer hashes are
  distinct from uncompressed DiffIDs; plain tar descriptors must equal their
  corresponding DiffIDs. No layer is fetched or decompressed.
- Raw bytes, environment, command defaults, history and annotations stay out of
  result summaries, Debug and fixed error values. No registry/Engine request,
  filesystem access, cache, process, write or new authority model is added.

Before parsing: 32 KiB per document and 64 KiB combined. Decode at most 16 index
entries and 64 layer descriptors/DiffIDs with bounded sequence visitors; refuse
extra elements rather than decoding unbounded collections. Media types cap at
128 bytes and platform names/variants at 64 bytes. The result summary stays below
2 KiB in the fixtures and contains no arbitrary source text. These raw-byte bounds
do not measure or replace whole-worker memory/time/FD/reply budgets.

`AdmittedImage` proves supported metadata consistency against the supplied root
digest only. The expected digest may itself be untrusted. This is not proof of
origin, signature, tag freshness, complete layers, full image/config validity,
local Engine availability, hardware compatibility, health or safe execution.
The four content gates remain unresolved. No serialized report can construct the
opaque result or become a protected source owner/approval/receipt.

## Root-pin correction and proposed version 2

The prior policy compared an immutable source pin with the selected manifest.
A multi-platform pin can identify an index, so those hashes can differ. A local
regression reproduced `Blocked` for that valid case before the fix and passes now.

- `ImageSource::Digest` uses `root_digest` in place of `manifest_digest`.
- `ImageResolution` and `ServiceTarget` retain `root_digest`, `manifest_digest`
  and `config_digest` separately. Pin movement checks the root; runtime change
  checks the config. Admission charges the additional digest before clones.
- Update review/post-comparison proposals now emit **version 2**. Frozen update
  settings remain version 1. No published route/schema/protocol is changed; these
  are unregistered leaf proposals. Future integration must register the v2 shape
  rather than combining a v1 digest-source reader with v2 output.

The byte-admission integration test supplies admitted index/manifest/config facts
to the v2 draft with an exact stack/source/platform context. It produces the
expected update proposal while all eight policy gates and four content gates
remain unresolved. Context/owner/authority cannot be manufactured by this test.

## Central integration requirements

1. Add `pub mod images;` beside executor-container deployment exports. Existing
   sha2/hex/serde dependencies suffice. This branch does not change shared crate
   roots, manifests/production lockfile, protocol, Store or worker registration.
2. The common protected collector must enforce acquisition limits before bytes
   reach this parser, resolve a reviewed allowed repository/tag or immutable root,
   and bind exact source/project/platform identity. Enforce approved origin,
   credential scope, redirects, transport/body/time limits and original ownership.
   This code has no network adapter and cannot authenticate supplied bytes/digests.
3. Collect within the existing worker's whole-cohort budgets. Bound aggregate
   image count, concurrent documents, elapsed time and retained reports; do not
   multiply this per-image ceiling into an unchecked batch allowance. Keep source
   owners/current identity and original sample times through revalidation.
4. Supply both the resolved root and selected platform manifest in v2 image facts.
   For a direct image they coincide; for an index they generally differ. Derive
   the runtime image ID from raw config bytes, never an index/manifest hash or
   downloaded-layer text. Refuse failed/unsupported admission; never guess missing
   values or relabel a report as a fresh protected observation.
5. Retain origin/signature/current registry policy checks and complete layer/
   Engine/runtime acceptance. Layer digests, DiffIDs and declared byte sizes here
   are metadata, not downloaded-content or disk-space proof. This helper does not
   implement a pull, unpack or image install operation.
6. Consume admitted data only under the existing current authority, common claims/
   locks, reviewed image/source targets, managed-file transaction, verified runtime
   comparison and uncertain-effect recovery path. Scheduled changes still require
   B1's separately reviewed revocable recurring policy. No one-time preview token,
   byte consistency result or serialized summary authorizes a mutation.

## Focused evidence and remaining acceptance

[Local packet](rewrite-evidence/p04/2026-10-15-image-content-admission/README.md)
binds exact code/fixture/harness hashes and lossless diagnostics, including the
root-pin regression's failing and passing runs. Synthetic OCI/Docker metadata
has independent Python hashlib hashes over the exact fixture bytes. Layer
descriptor hashes are placeholders; no layer content is present or qualified.

Focused tests cover supported direct/multi-platform chains, root/child/config
tampering, sizes/types, wrong/missing/ambiguous platforms, variants/qualifiers,
duplicate consumed fields, unsupported content, DiffID/count/size bounds, payload
privacy and the v2 policy boundary. B2 domain tests also run in this narrow
harness: 60 cases total (23 adapter, 25 update policy, 12 B2 domain), with strict
Clippy/Rustfmt and harness checks passing. API/UI/browser, genuine root-positive collection, registry/Engine,
whole-worker resources, effects/health/recovery, ARM64/Pi/package/performance and
cutover acceptance remain with shared integration and the qualification owner.
No Docker/registry/Pi command, host mutation or broad qualification suite ran.

References: [OCI index](https://github.com/opencontainers/image-spec/blob/main/image-index.md),
[manifest](https://github.com/opencontainers/image-spec/blob/main/manifest.md),
[config](https://github.com/opencontainers/image-spec/blob/main/config.md),
[descriptor](https://github.com/opencontainers/image-spec/blob/main/descriptor.md),
[Docker schema 2](https://distribution.github.io/distribution/spec/manifest-v2-2/).
