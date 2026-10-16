# Local HOST-001 planner evidence

15 October 2026. Source `92417d02735118f74e26a588625fb471bd73782f`, based on
main `b37c718`. Independent unregistered Engineer B leaf; HOST-001 remains open.

Final focused results: **13 Rust tests pass**, including **42 fixture scenarios**
(17 memory, 25 journal); strict Clippy, Rust formatting and Python harness
lint/format pass. Execution used x86_64 Rust 1.88 with one Cargo job, sequential
suites, on-disk caches/scratch and no host effects. This packet claims no installed
package, trusted collection, atomic write, API/UI, ARM64/Pi or reboot qualification.

`metadata.json` records source hashes checked against committed Git blobs, exact
commands/environment, tool versions, fixture counts and frozen reference hashes.
`harness-Cargo.toml`, `harness-lib.rs` and `harness-Cargo.lock` retain the independent
harness registration/dependencies. This lock is not the unchanged workspace lock.
`SHA256SUMS` binds the packet. Logs use lossless gzip compression. Final logs are
`test-final.log.gz`, `clippy-final.log.gz`, `fmt-final.log.gz`, `ruff.log.gz` and
`python-format.log.gz` (empty Rustfmt output means success, with exit status
recorded in metadata).

Two development regression pairs are retained: `kernel-separator-{red,green}.log.gz`
and `kernel-alias-{red,green}.log.gz`. They show unsafe append proposals before each
guard, then passing focused checks. These diagnostic runs predate the final
source and are not acceptance for it; their intermediate source snapshots were
not frozen as commits. The final full focused run includes both guards and the
additional platform/output limit. Initial Python lint reported a non-executable
shebang and import order; executable mode and formatted imports resolve those
tooling issues in the final committed harness.

`journal-cr-red.log.gz` records a third development failure: a comment followed by a
bare carriage return concealed a later empty assignment. Its code is `70dfc44`
with the final fixture file from the source above; the final run verifies refusal
of bare-CR syntax and continued CRLF support. This regression follows
[systemd's line-delimiter implementation](https://github.com/systemd/systemd/blob/v252/src/basic/fileio.c).

To reproduce, run the committed `tests/fixtures/deployment/prerequisites/check-leaf.py`
with uv, restore this harness lock in the prepared `.cache/prerequisites-leaf/`,
then run the recorded test/Clippy commands sequentially using the recorded disk
cache/scratch environment (adjust absolute worktree paths). Recompute source
hashes before crediting a different checkout. Shared manifests/exports are not
modified. The [handoff](../../../p04-host-prerequisites-handoff.md) lists the
required central integration and disposable-host acceptance gates.
