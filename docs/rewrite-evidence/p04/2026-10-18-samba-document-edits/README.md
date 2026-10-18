# Samba document editor — local source evidence

Product `67e127b4afdefa87faa022b7940a7a949ad70039`, branch
`engineer/p04-samba-document-edits`, base
`7df3f7530865c31b7faefd1f7b12a25aa7655860`.
The [handoff](../../../p04-samba-document-edits.md) documents the byte-splice
contract, limits and required central semantic/runtime acceptance.

| Check | Actual result |
| --- | --- |
| Pre-change share filter | 14 passed, 129 other unit cases filtered |
| Final full domain unit suite | 164 passed, zero failed/ignored |
| Compile-fail doctests | 5 passed |
| New coverage included above | 21 unit cases and 2 doctests |
| Strict all-target Clippy | Passed |
| Changed-path Rustfmt and whitespace | Passed |
| Installed binary, API/UI, apply/reload/recovery, physical/resource/performance | Unrun |

Commands: `cargo test --offline --locked -p limeos-domain` and
`cargo clippy --offline --locked -p limeos-domain --all-targets -- -D warnings`.
Rustfmt checks edition 2024 with `skip_children=true` on the three changed paths.
One Cargo job/test thread, disabled incremental compilation and repository disk
scratch/cache were used, sequentially. Toolchain and settings are in the manifest.
The initial full test run also passed, before replacing line-lowercasing with
fixed-prefix comparison; its log is preserved without adding repeated counts.

The actual domain module tests frozen imported rendering, append/update/remove,
byte equality outside the owned span, LF/CRLF/mixed surrounding styles/EOF,
stale current bytes, known ownership and refused adoption, marker corruption,
manual owned-block changes, conservative name collisions including disabled and
removed shares, bracketed parameter/comment values, suffix context, continued
logical headers/parameters, ambiguous continuation refusal, text/schema/byte/
line/section bounds and private debug/summary output. Compile-fail tests prove
no JSON construction and no source-lifetime escape. They do not prove file
provenance, safe global/include/registry semantics or installed behavior.

`inputs.sha256` hashes the domain, all tracked fixture inputs (a conservative
superset), workspace/lock/toolchain and Cargo settings. `reference-inputs.sha256`
binds the frozen plugin/test at `80593b29443ea9b81eb60ed4e5c3c3ac7a236204`.
The existing Samba fixture manifest remains unchanged. Frozen Python was read
only, never imported or executed. The inspected official
[configuration manual](https://www.samba.org/samba/docs/current/man-html/smb.conf.5)
and [parser notes](https://www.samba.org/samba/docs/old/Samba3-Developers-Guide/parsing.html)
inform conservative syntax refusal; neither qualifies the actual installed
Samba version. Opaque operator directives remain unresolved runtime inputs.

Changed source is limited to A-owned shares and its child module/tests, plus
the committed design. Shared crate roots/contracts, grants, Store/schema,
protocol, worker admission/supervision, routes/UI, packages and services are
unchanged. Cargo.lock and manifests are unchanged. The source list was bound
after committing unchanged final tested code. `artifacts.sha256` binds raw
compressed logs, manifest/script, source/changed lists and exact lockfile.

The proposed bytes cannot publish themselves. Genuine protected ownership,
complete configuration/dependency authority, actual testparm validation, atomic
apply/fsync, verified reload and real rollback/restart remain open. No A3/P04,
defect, physical, resource or cutover acceptance follows; the existing inventory
p95 61.401 ms against 20 ms remains failed. No production host was contacted.

Verify artifacts here with `sha256sum -c artifacts.sha256`. Verify inputs and
reference inputs from the worktree root using the packet's checksum paths.
