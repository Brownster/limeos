# Strict Compose JSON sources: local acceptance

Qualified source `2c68557c112b4caf10fe0f502a73580edfd7f411`, based on local main `23e5564`.
The pure adapter admits actual closed-subset JSON byte slices, combines operator/
managed declarations and re-admits compiled overrides against expected projects.
It acquires no file descriptor, lease, management authority or executable approval.
See the [design](../../../plans/2026-10-14-compose-json-source-design.md) and
[interface guide](../../../p04-compose-json-source.md).

**836 Rust cases pass:** 801 unit/integration cases and 35 compile-fail doctests.
The 23 new cases and focused rerun are already in that total. Full workspace
checks enable the existing opt-in local worker fixture feature.
**61 local ARM64 harness tests and six independent offline parser cases pass**,
with strict feature all-target Clippy, fmt, contracts and repository checks.
[checks.json](checks.json) binds commands, environments, source hashes and complete
merged stdout/stderr. Accepted source stays unchanged through documentation edits.

21 unit cases cover compiler artifacts and actual operator bytes, scalar defaults/
inheritance/false overrides, unrelated services, single managed-service removal,
shutdown requirements, every unresolved gate, stable plans and whitespace-sensitive
raw hashes. Unknown/duplicate/escaped/null/wrong/missing fields refuse at all supported
levels. Canonical u32 users/published ports, explicit mount modes, literal bind/device
paths, no host-directory creation, closed permissions/network modes, volume references,
unused definitions, image pinning, target and wildcard-port conflicts pass/refuse as
specified. Invalid overridden operator data cannot disappear through a managed file.

Bounded visitors refuse excess maps/arrays before the next value is decoded and
strings before owned clones. Exact 64 KiB raw inputs pass; excess, trailing values,
invalid UTF-8 and general YAML refuse. Combined 16/17-service, 8/9-resource and
128/129-volume boundaries refuse excess without truncation; equal port keys,
same-kind mount replacement and repeated standard declarations fit full sets.
Borrowed summary counting accepts its exact boundary and refuses without increment.
Two compile-fail cases prevent owner/report import. Parsed facts remain declarations.

Six [shared fixtures](../../../../tests/fixtures/compose-source/conformance-cases.json)
check new service emission, mount/port/device merge deltas, false scalar override,
single managed-service removal, host mode and scalar updates preserving sequences.
Rust verifies each artifact against the compiler and source combination. The
independent standalone parser runs only `version`/`config`; actual canonical JSON,
commands, version output/stderr, fixed environment and binary hashes are lossless
in [compose-conformance-accepted.log.gz](compose-conformance-accepted.log.gz).

The observed AMD64 parser is version 5.3.0, SHA256
`4727d85c084b042897384b58f65e767fb5d2e98a4783acc6380f544a59c68b32`. The same hash is checked before and after each case.
Explicit scratch files/project, cleared environment, disabled default `.env`,
environment-file/interpolation/path resolution and a nonexistent Unix endpoint
keep daemon effects outside the test. Invalid ambient `.env` traps coexist with
passing checks; private credential directories stay empty and source files unchanged.
Publisher/protected production launch trust, other versions, ARM64 installation,
general YAML and complete frozen-layout support remain unqualified.

The initial 18-case focused run and six-parser-case run passed. Final review added
full-set key replacement and broader visitor boundary assertions; the reviewed
19-case focused run passed. Final review reproduced two Serde format loopholes:
derived structs accepted array representation (including an empty volume array),
and unit enums accepted tagged objects (including object-shaped restart). The two
failed targeted logs and passing corrective logs remain lossless. An object-only
map visitor now guards every Compose object boundary; a bounded string-only enum
visitor guards restart/network mode/port protocol/mount kind. The hardened 21-case
focused run passes. All earlier passed full-source checks remain diagnostics;
final acceptance runs against the exact hardened source. No failed check was
discarded. The implementing agent reviewed the code; independent review is pending.

Limits are 64 KiB per source, sixteen services/eight ports/mounts/devices and 128
standard volume declarations across the combination, with 192 KiB counted borrowed
summary. These are admission ceilings, not peak allocator/RSS or installed headroom
proof. Bodies are dropped; hashes establish neither freshness nor recovery custody.
The existing compiler's unresolved gates remain unchanged. Source-to-retained-file/
Engine binding, physical completeness, fresh approval, claims/locks/writes/recovery
and verified shutdown remain open. No CLI/RPC/route/schema/Core/Store/package/service/
worker activation or dependency change is included. The signed candidate is unchanged.

Build/cache/general scratch and offline parser fixtures stayed on disk. Existing
storage fixtures compiled there before small owned auto-cleaned tmpfs runtime scratch.
50 ms samples observed lower-bound peaks of 57496 bytes/8 files,
not exact maxima. No Pi, SSH, production Docker, raw block probe, guest, reserved
backup or external qualification path was used. Guards remain 256 FDs/64 MiB/zero
swap/16 tasks. Historical inventory p95 61.401 ms still exceeds 20 ms; no P04/defect/
installed/resource/performance/cutover gate closes.

[source-manifest.json](source-manifest.json) binds 346 source/fixture/
contract/script files. Fixed-timestamp gzip retains every check log byte with raw/
compressed hashes; [artifact-sha256.json](artifact-sha256.json) binds the packet.
Reproduce against the qualified source using retained helpers and fresh work/log/
output paths. Compile storage fixtures before their small tmpfs runtime run.

Next: integrate supported byte admission with genuine retained catalog/file/Engine
owners under original bounds, then issue a fresh approved lifecycle operation
through the existing locks/recovery boundary.
