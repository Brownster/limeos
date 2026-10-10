# RW-040 CI probe supply source review

Reviewed 2026-10-10 by `confinement_review`, source/Git/primary-documentation reads only. No reviewer build, test, guest, Docker, SSH, host or Git mutation occurred. Prepared worktree: `/tmp/limeos-rw040-probe-artifact`, based on `1320dae0f0b12860cf901b1e4f57cf72155915cc`.

**Source GO for the finite CI-only producer seam.** This covers one workflow edit, the standard-library supply helper, its negative fixtures and supply documentation. Runtime/library/service/probe source is unchanged. The first actual CI build and downloaded artifact are still required evidence, not implied by this review. No gate closes.

The AMD64 workflow builds package executables and the `--locked --release --no-run` public integration test in the same content-addressed native Debian 12 builder image. It selects the exact Cargo `compiler-artifact` by test target name/kind/source/package manifest, release profile and confined release executable path; publication requires exactly one successful `build-finished` and one executable. The review identified Python boolean/numeric equality accepting `success=1`; the writer corrected this to require actual `True`. I read the explicit draft/mutation capture: the new numeric-success regression fails before correction and the seven focused tests pass afterward. This was an unfrozen producer draft, not alteration of frozen historical source.

The helper verifies clean exact workflow HEAD, unchanged public-probe SHA-256, selected executable bytes/mode, native AMD64 ELF, Rust 1.88.0, build glibc 2.36, expected GNU loader, GLIBC requirements no newer than Debian 12 and a closed supported dependency set. It records the actual compiler stream, compiler version, ELF inspection, image ID, lockfile/source-input hashes and all three selected package hashes/control fields. Same-head package attribution comes from the recorded workflow's actual build/package sequence, not from pretending package metadata alone proves source provenance.

I compared the resulting eight-field manifest with the real reviewed harness `verify_probe` parser. Its version, source/binary hashes, fixed executable name, toolchain, release profile and exact package `library_source` are compatible. The unchanged producer hash is `3929400eb457937923311206201e4ef8ce2b935afc685ec7e81b27e557699b9c`. No auth, PID, clock or profile substitute is introduced.

The fixture tests use explicit synthetic Cargo/ELF inputs to exercise negative admission: missing/duplicate/unfinished/wrong artifact, wrong source/package/profile/path, incompatible architecture/GLIBC/dependencies, changed published files and symlinks. They do not certify real compilation. That distinction is retained in the documentation.

External acquisition must bind the actual run/head, artifact IDs/names/API digests and saved ZIP bytes, inspect the expected members, then compare each supplied file/package hash with the build evidence. This happens after upload and is not fictitious pre-upload provenance. GitHub artifact API exposes run/head and digest fields; ordinary zipped uploads lose executable permission. The existing reviewed harness intentionally assigns its trusted input binary mode 0755 when creating the guest input archive, so that transport property does not require a producer redesign.

Primary references: [Cargo artifact and finished-message contract](https://doc.rust-lang.org/cargo/reference/external-tools.html#json-messages), [pinned upload action permission behavior](https://raw.githubusercontent.com/actions/upload-artifact/043fb46d1a93c77aae656e7c1c64a875d1fc6a0a/README.md), [GitHub artifact identity API](https://docs.github.com/en/rest/actions/artifacts#get-an-artifact).

Final local capture is `/home/marc/Documents/github/lime-os/target/rw040-probe-artifact-proof/source-final.json`. I read the final source and hashes, the focused seven-test OK log and full 22-reference-test OK log with recorded exit 0; the record also reports Ruff/format/ShellCheck/repository exit 0. These checks were executed by the writer, not this reviewer. Final reviewed hashes match the capture:

```text
a6e58d1b93eca76311c085888c7ed2c14c3ee800b399572bc0cc5395af4fc277  .github/workflows/ci.yml
81c31ff05a361235702031af9c43b3d2dca9798a8e474a8b64c029ebe2f0cfd0  scripts/build_rw040_probe_artifact.py
4fa0549c227881c0ce6078ad0a538de8c52c64cf0b82a4ecf2567b935d134d1c  tests/reference/test_rw040_probe_artifact.py
980def383c55bbdb14fad25a202a28e1d92b01f914fe1be95e1eefbcaf416498  docs/p04-rw040-probe-artifact-supply.md
```

Scope limits remain: no actual privileged public-library run, positive process/root/namespace qualification, lower-profile or combined capacity, worker supervision, restore effect or production capability decision. Only separately approved U0/B0 standalone guest runs may consume a genuine verified artifact.

## Authorized native harness guard registration amendment

The root authorized the final workflow/doc amendment after the original source GO. I read the two-line added native `check` step in `/tmp/limeos-rw040-probe-artifact`: `python3 -m unittest discover -s tests/qualification/rw040-combined -p 'test_*.py'`. It runs the complete discovery pattern from the repository root on both existing Ubuntu AMD64/ARM64 matrix entries using existing Python tooling. It introduces no test-name filter, forced count, QEMU/guest/Docker launch, permission change, secret, or package/probe source divergence. The 41 count comes from the previously read real corrected-harness capture, not the workflow text. Integration must include the reviewed full harness; the writer reports it integrated at `8ba41109` before this producer is combined. Existing source GOs apply unchanged.

**Amendment GO.** Refreshed `source-final.json` and independently read file hashes match: workflow `f00e552343624fb064bd31e8da94ef4854c5933a517a0c809b528ec8a1a92bc5`; supply document `676f792dabd830f162a2a7b4acb5f7ade1f6cb0a24ab02c0eaf2d536a80e0911`. Helper and producer-test hashes remain exactly the earlier reviewed values. No test or full source-review repeat was performed.
