# Current schema-9 ARM64 preparation

Local preparation passed. Native execution remains pending the operator's exact
SSH target and current start/end window. No Pi host was accessed and no installed
upgrade or performance result is claimed. Follow the
[current runbook](../../../plans/2026-10-12-arm64-schema9-qualification-runbook.md).

Branch `engineer/arm64-schema9-qualification`, worktree
`/tmp/limeos-arm64-schema9-qualification`. The
[design](../../../plans/2026-10-12-arm64-schema9-qualification-design.md)
freezes runtime `1401c255d4a05b8621c2dcd497d6f7f0cbe91d56` (reviewed runtime
implementation `8288315`, previously recorded native CI with 444 Rust cases).
No runtime, shared authority fixture or unmerged library/recovery work is
included in these harness changes.

## Final frozen bundle

| Input | Value |
| --- | --- |
| Fixture commit | `8b7ff58c5cd9cd0279e3ea73bd5904a41d15cad7` |
| Candidate version/schema/architecture | `0.4.8+arm64.1` / 9 / ARM64 |
| Bundle cache path | `.cache/arm64-qual/schema9-2026-10-12/guarded/source.tar.gz` |
| Bundle SHA-256 | `a24da0ce098cfaf74e91dced22a9640d4f0a1ac5de87b7144e70e23daca8b1a2` |
| Bundle size | 6,256,087 bytes |
| External manifest SHA-256 | `4ead49a69a63f87355c61de070b4b015013ef5deb25f0884c9361fda8f19aa6c` |
| Verified files | 1,647 runtime source, 39 fixture, 3 frontend dist |
| Shared upgrade fixture SHA-256 | `016ff3c25119f79b4eba3c95651ca63d56aa7f96c69a90bd57523fcc739d6893` |

[Manifest](guarded/source-manifest.json),
[bundle verification](guarded/bundle-verification.json) and
[frontend commands](guarded/frontend-logs/steps.json) bind the frozen source and
separate committed fixtures. `npm ci --ignore-scripts`, frontend tests (2 files),
TypeScript check and production Vite build passed against archived source.
The bundle was independently extracted and verified, including schema 9 and
internal/external manifest agreement. Native ELFs and packages do not exist yet
for this candidate; the native build must create and verify them.

An earlier bundle with fixtures `7ce2d98` and SHA
`91f28d97c5f5786f0e9687a3c5a35af8d673a554523d28cb485a56f109d7b5eb`
is retained in the parent cache directory and packet root. It was superseded
when the isolated-host window defect was found. It was never installed or run
on a host; use `guarded/` for qualification.

## Genuine previous artifact

[Provenance](previous-schema8-artifact.json) and
[actual-byte verification](schema8-verification.json) bind original source
`8acac403e32d3692b028a036a11f4fead2050580`, schema 8 and ARM64 package
`0.4.7+arm64.1`. The retained build record matches the reviewed historical record
SHA `8b6fe1dd64e3e4fedc454cc5a3ddbc630d5218bf5de187d285f56e774f31c50a`.
All 24 indexed historical evidence files and 557 original source files match.
The source's schema constant is 8. Both retained standard/shadow package bytes,
repository copies and controls match their original record; all four binaries
in each payload match SHA/size and actual ELF64 little-endian AArch64 machine
183. No old binary was executed and no package was rebuilt or relabeled.

The unchanged standard package is copied to
`.cache/arm64-qual/schema9-2026-10-12/upgrade-inputs/schema8/` with provenance.
Its package SHA is
`6c86199b253e155c94d96e1395c069c4188e11a2379f14eff97eacab0203b898`;
core SHA is `aab46a601a95924c8900a93749cd8cf5cc735cc2328783196732c363f258cde0`.
The expired original test key must be replaced by a fresh disposable guest
signature around these unchanged bytes before installed upgrade tests.

[Verification script](verify_schema8_artifact.py) records the exact local
read-only input checks and exclusive output writes. Its first attempt assumed
the standard binary directory for the shadow package and refused; that helper
error is retained in `schema8-verification-first.txt`. The corrected check uses
each actual package profile's binary directory and passes in
`schema8-verification.txt`. It establishes artifact provenance, not installation.

## Local regressions and limits

[Final harness log](harness-guarded.txt): 41 tests pass in 10.730 seconds. Eight
new test methods cover historical schemas 6/7/8, incomplete/changed provenance,
duplicate/missing/symlink artifacts, package bytes and controls, exact schema-8
dispatch/result binding, invalid dispatch refusal and explicit isolated-host
window/deadline enforcement. The upgrade operation is mocked in local wrapper
tests; the installed preservation fixture remains unchanged.

`isolated-window-first.txt` reproduces the actual runner defect: supplied windows
were ignored outside the production list. The fix validates any supplied record
before resolution and preserves its exact-target bounds, per-dispatch timeout,
deadline and cutoff refusal. Default production-host refusal still applies.
Local tests do not replace a current native supervisor proof.

Ruff lint/format and repository dependency/hygiene checks pass in
`ruff-guarded.txt`, `ruff-format-guarded.txt`, `schema8-ruff.txt` and
`repository-check-guarded.txt`. Earlier lint and harness attempts are retained; the
initial Ruff type-exception finding was fixed before final checks. No full Rust
suite was rerun locally because the runtime is frozen and unchanged.

Tonight's native build/install, footprint, schema 8→9 preservation, crash
recovery, storage/container and teardown results remain untested here. Historical
inventory p95 61.401 ms against 20 ms remains failed. Native KVM, bare-metal and
matched Python measurements remain distinct. Backup, P04 and production cutover
gates stay open. Estimate 16 human hours, scope review 24; P04 remains 320/480.
Actual human/agent effort is unmeasured; local tool waits do not establish it.

[Preparation status](preparation-status.json) keeps these gates explicit;
[evidence hashes](evidence-sha256.json) cover this packet, excluding the index
itself. Local large archives/packages stay in the cache and are bound by the
recorded hashes rather than added to Git.
