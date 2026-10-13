# ARM64 package inputs built off the production Pi

Implemented in fixture revision `687ddb43d889d693bcd55c0bf6352c20347a98ef`.
The [local acceptance and handoff](../rewrite-evidence/arm64/2026-10-13-native-ci-af94f19-inputs/README.md)
records 61 passing tests, the verified offline input, frozen bundle and fresh
signed repositories. Installed qualification still requires a new host window.

The October 12/13 [native run](../rewrite-evidence/arm64/2026-10-13-current-016ba49-memory-guard/README.md)
passed the current detached deadline proof but stopped both compilation attempts
at the host swap safeguard. Production services remained up and every guest was
removed. A 512 MiB build guest also reached the guard. Do further compilation on
native ARM64 CI or a separate build host; reserve Pi windows for installed tests.

## Prepared artifacts

The completed main CI run `38302065106`, source
`af94f19ac21f9da14b207439808ffb33c87e8e10`, has successful native ARM64 source gates
and Debian 12 release builds. Original artifact `11770788524` was downloaded and
matched GitHub's ZIP digest. Every original package control and all four AArch64
ELF hashes were checked. The 290 source/fixture/contract/script bindings from the
`016ba49` integration also match the frozen CI source, whose actual identity
continues to be `af94f19`.

New standard/shadow `0.4.8+arm64.3` packages were assembled on the workstation
using the frozen `af94f19` builder, unchanged native CI ELFs and original CI UI
bytes. Generated contracts match the same source. The original CI packages and
genuine previous schema-8 package remain unchanged. Exact package, binary and
input hashes are in the report's candidate provenance. These artifacts are
test candidates, now surrounded by fresh disposable signed repositories, with
no installed qualification.

## Implemented harness boundary

An explicit native-CI build provenance path now supports ARM64 fixture
preparation and validation. The existing native-guest build path keeps its
required gate set. CI artifacts never acquire fabricated `apt`, `rustup` or
other guest-build success steps just to satisfy the recorder.

The new input must bind the exact repository/run/job/source, successful native
architecture and required source/release gates, original artifact metadata and
ZIP SHA-256, original package controls and binary hashes, frozen package-builder
inputs, actual new package bytes, complete source/frontend/fixture manifests and
fresh disposable signing identity. Raw CI observations stay available. Validate
whole input identity and reject mixed source, architecture, controls, hashes,
missing gates and malformed or ambiguous provenance before any guest install.

Use the existing guest fixtures for actual installation, imported authentication,
service ownership/capabilities, dormant targets, receipt/claim recovery and
footprint. Continue using the genuine original `0.4.7+arm64.1` schema-8 package for
upgrade; current-source revision labels never replace historical provenance.
The recorder must distinguish native CI compilation, local packaging, KVM
installation/measurement and untested bare-metal behavior.

Local regressions cover changed records/bytes, missing or duplicate CI gates,
cross-source identities, architecture, package payloads, signed indexes and
Release expiry, plus the original build-validation path. Preparation independently
reconstructs every standard/shadow package file, mode, owner and control with
the frozen builder and unchanged CI ELFs. Actual package bytes remain bound by
SHA-256. The frozen signing script's partial `Release` self-entry is checked
against its generated header after validating both complete signatures.

The committed fixture revision, source-bound bundle and fresh repositories
around unchanged previous bytes now pass local validation. Caches and artifacts
remain on disk. The next step is installed qualification within a new explicit
target/window. The retained QEMU/image setup needs no repeated host installation.

Runtime/schema, service ceilings, capabilities, authority/claims, restore effects,
production Docker/storage and the frozen app are outside this harness change.
The expired October 12/13 window authorizes no new host dispatch. Install,
footprint, genuine upgrade and interruption tests still require a new explicit
target/window. P04 and basic-cutover gates remain open.
