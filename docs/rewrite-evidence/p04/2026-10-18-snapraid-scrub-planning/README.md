# Numeric SnapRAID scrub planner — local evidence

Tested product `808a6fe9118ee1b959daf14c87b7ceaf34fb941a`, branch
`engineer/p04-snapraid-scrub-planning`, base main
`7df3f7530865c31b7faefd1f7b12a25aa7655860`.
The [handoff](../../../p04-snapraid-scrub-planning.md) specifies API, preserved
option semantics, bounds and required central runtime acceptance. This packet
tests the actual domain crate; it is not an isolated duplicate implementation.

| Check | Result |
| --- | --- |
| Pre-change focused protection baseline | 11 passed; 132 other unit cases filtered |
| Final full domain unit suite | 157 passed, zero failed/ignored |
| Existing compile-fail doctests | 3 passed |
| New unit cases included above | 14 |
| Strict all-target Clippy | Passed |
| Changed-path Rustfmt / whitespace | Passed |
| Installed effects/API/UI, binary defaults, recovery, ARM64/Pi, resources/performance | Unrun |

The final command was `cargo test --offline --locked -p limeos-domain`.
Clippy used `cargo clippy --offline --locked -p limeos-domain --all-targets --
-D warnings`. Rustfmt used edition 2024 with `--check --config
skip_children=true` for the three changed Rust paths. Builds were sequential,
with one Cargo job, one test thread, disabled incremental compilation and the
shared target/cache and scratch on repository disk. Exact settings/toolchains
are in the manifest. No dependency, root manifest or Cargo.lock changed.

The final tests exercise both enabled frozen layouts, complete original source
bindings/rendering/time, all optional numeric combinations including zero,
saved/manual default separation, invalid numeric/free-form inputs, manual vs
recurring flags, every-role unsafe/missing/type cases, every content/parity
absent-file distinction from initial sync, incomplete/reordered/rebound paths,
exact TTL boundaries, every device's identity/boot/swap/read-only/bind/host-root
mount refusals, nested/ambiguous mounts, current filesystem support, pool roots
and generated leaves, and sync-threshold separation. No proposed preview is
credited as physical provenance, executable authority or successful scrub.

The first two full runs each reported 155 passes and one fixture failure.
One negative mutation swapped identical source/data root observations; the
other copied one over the other. Both were no-ops under the current evidence
shape. The corrected fixture selects distinct paths and asserts each mutation
actually changes evidence. No production planner correction was needed.
Those logs are retained; their counts are not added to the final total.

`inputs.sha256` binds tracked domain source/tests, pool/protection fixtures,
workspace/lock/toolchain and Cargo settings. `reference-inputs.sha256` hashes
the frozen Python plugin/helper, parameter/configuration schemas and bundled
manual at reference HEAD `80593b29443ea9b81eb60ed4e5c3c3ac7a236204`; these files
were read only and never executed/imported. The inspected current
[official manual](https://www.snapraid.it/manual) corroborates numeric options
and content metadata writes, without qualifying the installed binary/version.

The changed-path record contains only A-owned protection source, its child
module/tests and the committed design. Crate-root exports, generated contracts,
read queries/workers, grants, Store/schema/protocol, package/service activation,
API and UI are untouched. Root domain compilation does not register a runnable
scrub operation. Shared admission, source owners, claims/locks, authorization,
supervision, content recovery/receipts and installed acceptance remain open.

`artifacts.sha256` binds the raw compressed check/failure logs, manifest,
packaging script, changed/source lists, toolchain records and exact lockfile.
No source changed after final checks before the product commit. No production
host was contacted. A3/P04/SRA defect, hardware, installed recovery and cutover
gates remain open; the existing 61.401 ms / 20 ms inventory gate remains failed.

From this directory run `sha256sum -c artifacts.sha256`. From the worktree root
verify the packet's `inputs.sha256` and `reference-inputs.sha256`.
