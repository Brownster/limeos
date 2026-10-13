# ARM64 package inputs built off the production Pi

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
unsigned test candidates with no installed qualification.

## Next bounded harness work

Add an explicit native-CI build provenance path to ARM64 fixture preparation and
validation. Keep the existing native-guest build path and its required gate set
unchanged. CI artifacts must never acquire fabricated `apt`, `rustup` or other
guest-build success steps just to satisfy the current recorder.

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

Local fixture regressions should cover the new record's refusals and preservation
of the original build-validation path. Then freeze the new fixture revision,
prepare a source-bound bundle and fresh signed repositories around unchanged
previous bytes and the new candidate. Keep all caches and artifacts on disk.
Only after those inputs pass local validation should another Pi test window be
requested. The retained QEMU/image setup needs no repeated host installation.

Runtime/schema, service ceilings, capabilities, authority/claims, restore effects,
production Docker/storage and the frozen app are outside this harness change.
The expired October 12/13 window authorizes no new host dispatch. Install,
footprint, genuine upgrade and interruption tests still require a new explicit
target/window. P04 and basic-cutover gates remain open.
