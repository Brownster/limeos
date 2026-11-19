# Pi night run 2026-11-07: cross-built candidate rebuilt from current main (nothing run on wybie)

**Status: prepared, not run.** No SSH, no VM, no ARM64 code executed. Built and checked on the workstation only.
Method, guards and untested list are unchanged from the [2026-11-05 preparation](../2026-11-05-pi-night-prep/README.md);
this packet records only the new candidate. The harness scripts here are the November 5 ones; the only edit is the
key label (`night 2026-11-07`) and a 30-day key lifetime (`30d`, was `7d`).

## Candidate

| Item | Identity |
| --- | --- |
| Runtime source and fixtures | `3dc51cabc2b3b3de0704369a3745e72a145e820f` (main), authority schema 16 (was 15) |
| Package version | `0.4.17+arm64.1` |
| Build | `build_arm64_binaries.py 3dc51ca`: offline `cargo build --offline --locked --release --target aarch64-unknown-linux-gnu`, image `sha256:af684f63…5ba5`, `CARGO_BUILD_JOBS=4`, exit 0. [Record](inputs/cross-build-record.json), [log](EVIDENCE-BUNDLE.json "bundled: inputs/cross-build-output.log.gz") |
| Bundle | `make_bundle.py --cross-build --commit 3dc51ca --fixtures-commit 3dc51ca --package-version 0.4.17+arm64.1 --authority-schema 16`; frontend `npm ci --ignore-scripts`, `npm test`, `npm run build` all exit 0 ([steps](inputs/frontend-steps.json)) |
| `source.tar.gz` | `782126733d6640bdcd165e5cb52367514a46ffb5b5f707da5914b5a0c378dc8f` (57,712,845 bytes) |
| `source-manifest.json` | `df4afd37e46dd7dfaf7412062641eb960224e00eba4a5c7efb4f616a7f6fd14b` |
| `build-result.json` | `60d953a5a9d3fd64bfc94ed440af0af07c517a26084e09d818af477ac1d17138` (kind `local-cross-build`) |
| `candidate-summary.json` | `af30a1e648ff86b5dfe0d06e90217fc7a2957fdc9cd670213ad880b02f4bbd2d` (all repository file hashes: [copy](inputs/candidate-summary.json)) |
| Standard `.deb` | `limeos_0.4.17+arm64.1_arm64.deb` `be182a7c802a14c4cbc38a6e988403764557c0200e84ce7a59596f0c70efb473` |
| Shadow `.deb` | `limeos-shadow_0.4.17+arm64.1_arm64.deb` `b9814cd65760f9433521b67905d056f83f6d337e6cf3efc3afd92f1079f4a006` |
| Signed repository | throwaway ed25519 key `CF5A886ED2DEB6E624BF1FA918E2DFC28BF247EA` ("LimeOS disposable arm64 night 2026-11-07"), **expires 2026-12-06**; `InRelease` `327dcf2e4c9673a7418b16ca20324246e376449443ec198594685f34bba7be53`, verified with `gpgv`. Private key stays in `.cache/q/signing`, uncommitted |

| Binary (aarch64) | SHA-256 | Bytes |
| --- | --- | --- |
| `limeos-core` | `0ffc0ee9beb03013b1d68ad5026c377159c1290ef01b236db264d587750c5c4e` | 7,032,760 |
| `limeos-executor` | `f519ea36af6eb0ec453222e45eadfd7d41bf2ecfaa83d62007d691952bca8bdc` | 5,459,640 |
| `limeosctl` | `4a16d0a4b52816aa04a30809987dbdcfeb0bc9abe662eb3f3c43ba474bbc9d7b` | 1,184,464 |
| `limeos-password-worker` | `896314be6fd7a7869759d19c0d1d739f92f55f852f1d38db778a815ab7c47af0` | 529,024 |
| `limeos-stack-read-worker` | `28d7d996e8237a80919eea8d98307f6aee2b37e6f90756ac9e79abcf0f5656f8` | 1,118,928 |

On disk (not committed): `.cache/arm64-qual/2026-11-07-cross/{source.tar.gz,source-manifest.json,candidate-summary.json,candidate/}`
in the worktree `.claude/worktrees/agent-a9b3906f1084cbe11` (copy into the main checkout's `.cache` before the run; the
Debian image is reachable at `.cache/arm64-qual/integrated-2026-10-12/` in main, SHA-512 verified by the script).

## Changes since the 2026-11-05 candidate (df24c29 to 3dc51ca, product paths)

Job resolution end to end (domain types, schema 16 persistence, executor `ResolveAction`, core/API resolution of
`needs_intervention` jobs, `limeosctl job show-resolution`/`job resolve`, frontend Needs-attention banner and dialog,
regenerated contracts); `limeosctl migrate propose-catalog`; cutover activate with automatic rollback and HTTPS proxy
site; test-only container/protection-source fixes. Guest suites in the bundle are those at 3dc51ca (whole-stack
lifecycle follow-ups included). Schema moves 15 to 16, so no upgrade-from-previous coverage (as before).

## Checks run here

- `make_bundle.py --cross-build`, binary build and packaging exit 0; two-compilation byte-identity was not repeated.
- `tests/qualification/arm64`: `python3 -m unittest test_pi_night` 26 tests OK.
- `run_pi_night.py` without `--execute` (no SSH): exits 0, `problems: []`, with a draft authorization (not committed).
  The check requires the window to have started and be at most 12 h, so it was run (a) with the real clock against a
  window 2026-11-06T17:00Z to 2026-11-07T05:00Z, deadline 04:30Z, and (b) with the clock patched to 2026-11-07T01:00:05Z
  against 01:00Z to 09:00Z, deadline 2026-11-07T08:30:00Z (the exact tonight parameters). Both pass.

## Command for tonight

```
uv run tests/qualification/arm64/run_pi_night.py --host holly@wybie --authorization AUTH.json \
  --deadline 2026-11-07T08:30:00Z --candidate .cache/arm64-qual/2026-11-07-cross/candidate \
  --run-dir .cache/arm64-qual/night-2026-11-07 \
  --image .cache/arm64-qual/integrated-2026-10-12/debian-12-genericcloud-arm64.qcow2 --execute
```

`AUTH.json` is the operator's real window (window end at least 20 minutes after the deadline, at most 12 h long).

## Caveats

Cross-built: no native test totals or lint gates; the cross toolchain, not the Pi, produced the ELFs. Nothing here
was executed on ARM64. Docker Desktop was started for the build and stopped afterwards.
