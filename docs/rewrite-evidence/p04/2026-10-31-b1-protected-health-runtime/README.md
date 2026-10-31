# Local protected container-health runtime qualification

Qualified source: `1219844171eee78bd897458295681c1892b4be7f` on local main.
Its parent, storage correction `245d53d`, fixes the Btrfs source failure exposed
by this run. The source-bound
[handoff](../../../p04-b1-protected-health-runtime-handoff.md) explains the
worker/RPC, original freshness owner and partial Store consumption. The
[design](../../../plans/2026-10-31-b1-protected-health-runtime-design.md) defines
the intended boundary. No normal scheduler or installed service was activated.

## Final source acceptance

| Check | Result |
| --- | --- |
| Whole Rust workspace, `limeos-executor/read-worker-probe` enabled | 1,701 runtime passes, 53 compile-fail doctests |
| Focused storage suite on the same repository Btrfs disk | 237 passes; included again in the workspace run |
| Strict Clippy, workspace/all targets with the same opt-in feature | Pass, warnings denied |
| Actual contract regeneration/drift | Pass; four generated outputs changed |
| Frontend serial tests and checked production build | 242 passes; build passes |
| Read-only package/profile reference tests | 13 passes |
| Formatting, repository boundaries, changed Python Ruff checks | Pass |
| Actual `postinst` ShellCheck 0.10.0, explicit file argument | Pass |

Three existing child-entry helpers are ignored by the outer Cargo harness and
invoked by passing isolated-process parents: the executor's journal-capacity
probe and runtime bootstrap child, and storage's journal-descriptor probe.
Child output is not counted a second time. [Source-match metadata](source-match.json)
records each final Cargo group, exact commits and all attempt results.

Seven final guards have identical before/after maps covering 730 committed
inputs: `rust-4`, `storage-2`, `clippy-1`, `contracts-1`, `frontend-1`,
`reference-1` and `sanity-3`. Every recorded byte also matches the qualified
commit and final worktree. All checks ran serially with one Cargo build job,
one Rust test thread, repository-disk scratch/caches, a 4-GiB start threshold
and the unchanged sampled 3-GiB stop floor. The lowest final sampled available
memory was 4,809,543,680 bytes. No guard fired or external compiler overlapped.
The final native check finds all 32 original wrapper/child PIDs absent.

## What the local tests establish

Owned Unix listeners exercise actual Core framing, root-policy refusals before
request bytes, nonce/version/payload/time checks, socket replacement, original
deadline expiry and contention. Successful fixtures use private unit-only
credential and clock substitutions. Actual database worker and SQLite write-lock
waits prove expired work cannot gain a later admission or complete with a renewed
lifetime. Source wall time is checked again in the completion transaction.
Current concrete grants, legacy silence refusal, retained-subject retirement
refusal and partial incident deduplication run through real isolated Stores.

The opt-in transport suite copies/strips the actual built production worker,
seals it and invokes its health roles through the real supervisor. An isolated
observer sets only its own soft descriptor limit to the existing guardian
requirement. The parent limit stays unchanged. Production roles bootstrap and
return a collection refusal with confirmed cleanup under nonroot credentials;
this refusal alone does not prove the precise collection stage. Direct on-disk
roles/unsealed loading refuse. This is no successful installed UID-0 Engine test.

An owned native Btrfs probe records inode device `0:43`, mount-table device
`0:35`, matching mount ID 67, live link counts of one and detached count zero.
The separately committed source fix authenticates the actual proc table before
using filesystem-specific rules, corroborates final mismatched Btrfs descriptors
with kernel filesystem type and retains both actual device numbers. Forged
tables still refuse. Physical Btrfs binding remains unavailable and ordinary
physical/destination exact-device joins remain unchanged. The directory test
accepts live positive counts while preserving detachment refusal. See the
[source correction design](../../../plans/2026-10-31-source-btrfs-device-design.md).

## Preserved development attempts

All sixteen guard records and full logs are retained, including failures.

| Attempt | Outcome and interpretation |
| --- | --- |
| `rust-1` | Compile failure: existing AlertSignal lacked schema/TS derives needed by the new cohort; corrected |
| `rust-2` | Worker fixture failure; inputs changed during the run, so it supplies no final-source acceptance |
| `health_probe-1` | Actual inherited soft NOFILE 1,048,576 did not meet the guardian's 256 requirement; cleanup unconfirmed |
| `health_probe-2` | Isolated observer uses the required local limit; real sealed worker refusal/cleanup passes; earlier source map |
| `schema-1` | Actual emitter succeeds; four changed generated files copied, then independently checked by final `contracts-1` |
| `rust-3` | Seventeen storage failures expose the Btrfs device mismatch and directory-count assumption; no final workspace pass |
| `storage-1` | 235 passes, two fixture expectation failures after genuine-table authentication; corrected without relaxing physical binding |
| `storage-2` | 237 passes on final source |
| `rust-4` | Final whole-workspace pass |
| `clippy-1`, `contracts-1`, `frontend-1`, `reference-1` | Final passes |
| `sanity-1` | Environment failure: ShellCheck absent |
| `sanity-2` | Other checks pass; skill wrapper skips extensionless `postinst`, so this is not ShellCheck acceptance |
| `sanity-3` | Explicit actual-file ShellCheck plus other source checks pass |

The pinned linter came from the official koalaman/ShellCheck v0.10.0 HTTPS
release and was extracted only into repository cache. Its observed archive hash
is `6c881ab0698e4e6ea235245f22832860544f17ba386442fe7e9d629f8cbedf87`.
The release metadata supplied no upstream digest; local hashes are observations.
Provenance, exact commands, launcher identities, generated-output delta, frontend
asset hashes, post-run debug artifact hashes and process-absence checks are in
`launch-and-local/`. Post-run artifact hashes establish no package qualification.

## Reproduction and remaining gates

`reproduction-sources.tar.gz` contains changed committed source, both designs,
qualification helpers and the unchanged guard/wrapper sources.
`feature.patch.gz` and `storage-fix.patch.gz` preserve the separate commits.
`checks/` contains gzip-compressed complete guard records and logs;
`SHA256SUMS` covers every other packet file. Reproduction assumes the repository,
locked cached toolchain/dependencies and the pinned local linter. No scratch
directory belongs in `/tmp`.

Container-only samples remain globally partial. Legacy names and removed IDs
require explicit review, and normal due/authority preflight, queued recovery,
fair supervision and remaining host/SMART/mount/protection sources are pending.
New package-created storage policies include the narrow health flag; existing
policies are preserved. Older binaries require policy before-images restored
with the matching pre-schema-14 database and package assets for a return path.

Installed root-positive acquisition, current API/UI/restart behavior, full
ordinary notifications, retention/compaction, packages/upgrades/return paths,
ARM64/Pi, resource/performance and real 24-hour idle writes remain unqualified.
A1 reliability, broader A/B/P04 and P05/P06 cutover remain open; no defect row
or historical inventory-performance failure closes. Holly's Pi, real Docker,
the frozen app and external recipients were untouched. Nothing was pushed.
