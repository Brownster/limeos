# RW-040 container dependency acceptance

Frozen source: `63bcd78abfd1a0dd413183d3d2899d0aab4afa2c`. Runtime collection is introduced by `5201ae9`, with the installed response-handling correction in `2bd092d`. The [design](../../../plans/2026-10-06-storage-container-dependencies-design.md) estimates 12 engineering hours with review at 18; the phase remains 320 hours with review at 480.

[Validation](validation.json) binds the exact build bundle, all 225 source/build files, standard/shadow AMD64 0.4.6 packages, four binary hashes, unchanged frontend assets and installed result. Artifacts remain in `dist/p04-dependencies-fixed-debian`. Both profiles were installed through the signed disposable repository. The release build ran offline with Rust 1.88 in a fresh Debian 12 KVM guest and took 3 minutes 3 seconds. No Python checkout or production host was modified.

[The installed result](dependencies-vm-result.json) records 49 passing groups: all 38 existing container lifecycle/log/shadow groups, including 18 real crash scenarios, plus 11 dependency checks. The accepted inventory includes all 31 containers in the synthetic Engine, including a running unmanaged read-only bind and a never-started named volume. tmpfs is validated and omitted from host sources. Environment and labels remain private. HTTP authentication, no-query/no-POST behavior, no intent/audit/claim/Engine effects, unavailable and oversized evidence, changed enumeration, actual container state change, grant revocation during a read, independent read ceiling and shadow removal are exercised.

[The 199-test transcript](rust-tests.txt) includes domain path/age/identity limits, malformed mounts, unknown types, duplicate identities/destinations, collection races and deadlines, RPC admission, core socket ownership, error propagation, API authority and all existing storage/persistence regressions. Formatting, strict Clippy, generated contracts, repository checks, fixture lint/format, frontend checks and cached dependency/advisory checks pass. No migration, capability or third-party dependency was added.

[CI 37517732734](https://github.com/Brownster/limeos/actions/runs/37517732734) passes native AMD64 and ARM64 199-test suites, native release packages and all five AMD64 guests. [Its identity record](native-ci.json) binds the retained fresh results: foundation 14 groups, observations 6, representative layouts 61, approved storage/teardown 50 and dependencies/lifecycle 49. CI labels 0.4.4 and 0.4.4+ci.1 contain current source; they establish no genuine historical upgrade. Previous hash-bound old-payload upgrades remain separately recorded in the [P04 tracker](../2026-10-05-execution-tracker.md).

The earlier 0.4.5 candidate at `b346505` failed the changed-list check, locally and in CI. [Diagnosis](receipt-error-diagnosis.json), [CI excerpt](b346-ci-failure-excerpt.txt) and [guest diagnostics](b346-dependencies-vm-failure.txt) retain the failure. Core checked false readiness before reading the executor's specific error. The new regression fails with `Unavailable` instead of `Conflict` before the fix and passes after version-first, error-preserving handling. Accepted 0.4.6 artifacts have distinct identities; the failed candidate is not relabeled or attributed to them.

The local guest observes 12,148 KiB combined base-service PSS, zero swap and 0.762 ms overview p95 across twenty warm reads after lifecycle qualification and before the dependency scenarios. This is limited AMD64 VM evidence. Docker, fixtures, build/browser tools, optional storage readers/target service, password workers and assistants are excluded. No new Pi footprint or comparable Python claim is made. The ARM engineer's current assignment retains its earlier frozen base and independent qualification scope.

To reproduce from the frozen source with the frontend built and a verified local Debian image:

```sh
uv run tests/privileged_vm/build_bundle.py --output /tmp/limeos-p04-dependencies-fixed-build.tar.gz
uv run tests/privileged_vm/run.py \
  --repository dist/p04-approved-final-debian/limeos-repo \
  --build-bundle /tmp/limeos-p04-dependencies-fixed-build.tar.gz \
  --build-output dist/p04-dependencies-fixed-debian \
  --package-version 0.4.6 --retain-previous-repository \
  --guest-script tests/privileged_vm/p04_dependencies_guest.py \
  --output docs/rewrite-evidence/p04/rw040-dependencies/dependencies-vm-result.json
cargo test --workspace --locked --offline \
  > docs/rewrite-evidence/p04/rw040-dependencies/rust-tests.txt 2>&1
uv run tests/privileged_vm/record_dependencies_evidence.py \
  --bundle /tmp/limeos-p04-dependencies-fixed-build.tar.gz \
  --artifacts dist/p04-dependencies-fixed-debian \
  --source-commit 63bcd78abfd1a0dd413183d3d2899d0aab4afa2c \
  --evidence docs/rewrite-evidence/p04/rw040-dependencies
```

Retaining the previous repository supports other fixtures; this fresh dependency run deliberately does not execute an upgrade. Disposable guests require KVM and network access for Debian test prerequisites. Signing keys and disks are temporary and are removed when each guest ends.

Docker declarations and lexical impact calculations cannot prove absence of physical consumers. Protected alias and running-container mount resolution, named-volume backing evidence, pools, protection and shares remain pending. Mount/fstab effects, dependency-aware unmount, runtime-loss shutdown and guided React screens remain gated. No P04 defect-register row closes on this milestone.
