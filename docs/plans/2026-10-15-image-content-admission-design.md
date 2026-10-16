# Read-only image content admission

Engineer B, 15 October 2026. Follow container-update branch `d8f94de`; do not edit
shared roots or A's active mount files. This is bounded byte validation using
existing sha2/serde dependencies, not a registry client, protected collector,
resource registry, executable operation, approval or success receipt.

The previous update draft conflates a configured immutable reference with the
selected manifest. An immutable reference may identify an OCI index. Keep root,
platform-manifest and image-config digests distinct; match a pinned source against
the root. Bump the unregistered update summary proposal to version 2 and replace
the digest-source field with `root_digest`. Existing published runtime/API
contracts are unchanged. Add a regression before fixing this condition.

Admit one direct manifest or one index plus selected manifest and config from
borrowed raw bytes. Verify the root against its expected SHA-256, the selected
child's descriptor size/digest/media type, then the config's size/digest. Hash
original bytes, never reserialized JSON. Parse only admission-relevant fields;
skip runtime settings/environment/history/annotations without retaining values.
Reject duplicates of consumed fields. Opaque results expose only bounded digest/
platform/count/size summaries and outstanding integration gates, with no raw bytes
in Debug/error output. The expected root digest itself proves no trusted origin.

Support OCI image manifests/indexes/configs and Docker schema-2 equivalents for
linux/amd64 base v1 and linux/arm64 base v8. Require exactly one matching index
entry and agreement with config platform. Accept omitted base variants; refuse
higher variants, unsupported OS requirements, ambiguous candidates, matching
unknown/unsupported types, artifacts, nested indexes and external/embedded content.
Foreign-platform/attestation entries are never selected. This conservative subset
does not claim every conforming OCI document is supported.

Validate rootfs type/layer DiffID count/canonical digests against manifest layer
count and check layer descriptor syntax/media types/size-sum overflow. Do not
download/hash/decompress layers or compare compressed layer digests to DiffIDs.
No claim of layer integrity, runnable contents, signatures, full config semantics,
registry freshness, Engine availability or health follows from metadata admission.
Three byte blobs plus an unverified resolver report would not provide those
properties, so an effect-capable resolver is outside this slice.

Bounds before parsing: 32 KiB each raw document, 64 KiB combined, 16 index entries,
64 layers/DiffIDs, 128-byte media types and canonical SHA-256 identities. Bound
sequence allocation during decoding. Retain no runtime configuration or payload
bytes. Input acquisition and whole-worker/reply budgets remain central; a bounded
parser cannot repair an unbounded collector. Bad/unsupported content stays an
explicit failure and must not be replaced with guessed image IDs or pull logs.

Focused tests cover real byte-hash fixtures, OCI/Docker direct and multi-platform
chains, root/child/config hash/size/type mismatches, wrong/missing/ambiguous
platforms, duplicate consumed fields, unsupported qualifiers/URLs/embedded data,
layer/DiffID/count/aggregate limits, privacy and the root-vs-manifest regression.
Run local tests/strict Clippy sequentially with one Cargo job. Preserve exact
source/fixture identities and all failure logs. No Docker/registry/Pi commands,
layer fetch, host mutation, package/hardware/broad qualification or phase closure.

Primary references: [OCI index](https://github.com/opencontainers/image-spec/blob/main/image-index.md),
[manifest](https://github.com/opencontainers/image-spec/blob/main/manifest.md),
[config](https://github.com/opencontainers/image-spec/blob/main/config.md),
[descriptor](https://github.com/opencontainers/image-spec/blob/main/descriptor.md),
[Docker schema 2](https://distribution.github.io/distribution/spec/manifest-v2-2/).
