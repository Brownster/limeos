# Strict Compose JSON source admission and offline conformance

Base: local main `23e5564`. Add a pure, unregistered source adapter in the container
library, using existing Serde/SHA256 dependencies. Consume bounded byte slices;
do not open files, run commands, import reports or issue approvals. JSON is the
first explicitly supported source format. YAML anchors/tags, `.env`, includes,
interpolation and arbitrary Compose features remain explicit refusals.

Decode one operator document and optional replacement managed document, each at
most 64 KiB. Require a services map and optional volume declarations. Use closed
structs, bounded string visitors, bounded map/array admission before value decoding
or growth, and duplicate-field/key refusal including escaped equivalents. Reject
explicit null, trailing documents and unsupported keys at every level. Services
are at most sixteen; ports/mounts/devices at most eight per service; empty standard
named-volume definitions at most 128 across the combination.

Supported service fields are immutable image, canonical numeric `uid:gid`, restart,
privileged, host-only network mode, explicit long-form ports/mounts and literal
`source:target:rwm` devices. Omitted scalars retain documented defaults/inheritance;
null cannot clear them. Mount read-only values are explicit; bind options require
`create_host_path: false`. Reject custom drivers/external volumes, short ambiguous
ports/mounts, alternate device permissions, omitted/true host-directory creation,
and interpolation or nonliteral paths. Validate every declared service/volume,
including overridden/unused declarations, and apply existing P03 normalization.

Combine supported service maps, scalar overrides, unique port keys, mount targets
and device additions. Refuse kind switches and excess combined counts before
growth; preserve all other operator services. A referenced named volume must be
declared in the complete combination. Retain raw byte counts/SHA256 for each
document and normalized operator/current projects in a non-deserializable checked
source value with a borrowed serialize-only bounded summary. This is parsed data,
not a retained descriptor/lease, management ownership, physical consumer or approval.

Its planning method invokes the existing pure compiler, then admits and combines
the generated JSON again and requires the expected normalized desired project.
The compiler's unresolved gates stay intact. Before/after hashes have no recovery
custody; an empty desired project remains a shutdown proposal with no deletion
authority. Protected file acquisition and revalidation integration come later.

Local Rust fixtures cover compiler-produced artifacts, effective combinations,
numeric ownership, scalar false values, target replacement, inherited rows,
unknown/duplicate/escaped/null fields, limits before excess decoding/growth,
combined count/port conflicts, volume declarations, unsupported source syntax,
raw whitespace fingerprints, deterministic ordering and import refusal.

Add opt-in local qualification tests for the independently installed standalone
Compose `config` parser. Pin observed AMD64 version `5.3.0` and executable SHA256
`4727d85c084b042897384b58f65e767fb5d2e98a4783acc6380f544a59c68b32`.
Check the actual binary before/after each run. Use only explicit scratch files,
project name/directory, cleared environment, disabled environment-file resolution,
interpolation and path resolution, and a nonexistent Unix daemon endpoint. No
`up`, pull, build, image resolution or daemon request is involved. Preserve native
canonical outputs and compare supported service/volume facts to Rust fixtures.
This qualifies the selected parser/subset, not publisher trust, production launch,
other Compose versions, ARM64 installation or complete YAML support.

Commit design, implementation/fixtures and acceptance separately. Retain complete
logs and exact-source/artifact hashes for Rust, strict Clippy/fmt/contracts/repository,
local ARM64 harness and explicit native-parser conformance. Keep build/cache/general
scratch on disk; only existing precompiled small storage fixtures use cleaned tmpfs.
No Pi/SSH, production Docker, block probe, guest, reserved backup or external
qualification paths. Core/Store, locks/recovery, schemas, services, dependency
versions, signed candidate and activation remain unchanged.
