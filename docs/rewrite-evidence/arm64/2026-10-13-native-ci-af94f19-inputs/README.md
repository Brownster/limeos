# Native CI ARM64 inputs: local acceptance

The next installed qualification can use these prepared inputs without compiling
on the production Pi. All 61 harness tests pass locally and in a disposable
Debian 12/Python 3.11 container. The complete offline input validates against the
actual candidate packages and fresh signed repositories. This preparation made
no SSH connection, installed no Pi package and performed no installed, upgrade,
crash or performance test. The previous host window has expired.

Runtime source is `af94f19ac21f9da14b207439808ffb33c87e8e10`; fixture source is
`687ddb43d889d693bcd55c0bf6352c20347a98ef`. The runtime, contracts and operation
fixtures match the October 12 integrated candidate as recorded in the
[overnight report](../2026-10-13-current-016ba49-memory-guard/README.md).
The actual CI identity remains `af94f19`. The separate dependency-graph branch
and unfinished backup-bound recovery are excluded. P04 and cutover remain open.

| Input | Binding |
| --- | --- |
| Native CI run | [38302065106](https://github.com/Brownster/limeos/actions/runs/38302065106) |
| Native ARM64 job | [114997846707](https://github.com/Brownster/limeos/actions/runs/38302065106/job/114997846707) |
| Original artifact | `11770788524`, `debian-ubuntu-24.04-arm` |
| Original ZIP SHA-256 | `45462b1787cb95d1bc6fcfa4b3c8befa565f8552aaebd5d57833b91e3a9b3080` |
| Candidate version/schema | `0.4.8+arm64.3` / 9 |
| Bundle SHA-256 | `5f041f8f37fbfcc9568490015ba6fa464122e8ccf360808ebb282a70ccb76348` |
| External manifest SHA-256 | `fad3172e4b187a27cbf7c60f57af298559a21ef53422b6b186d9296ac1f17798` |
| Offline build-input SHA-256 | `da27cdd0efe29639f029ff01845a43a976aee16a7c7e76dfd14d92f3f8c2dc0c` |
| Standard `.deb` SHA-256 | `68cf2de38e7bb82656773103c61e5e938d30e4b4142495018412acbea2127b8a` |
| Shadow `.deb` SHA-256 | `b0b4aaa47fed3788fccda86b139cead9c4a046f72976a6fd5c727cc323b081cb` |
| Disposable signing fingerprint | `96213BC58271A79707C267C724633C5051BBE7A2` |
| Genuine previous version/schema | `0.4.7+arm64.1` / 8, source `8acac403e32d3692b028a036a11f4fead2050580` |
| Unchanged previous `.deb` SHA-256 | `6c86199b253e155c94d96e1395c069c4188e11a2379f14eff97eacab0203b898` |

The [source manifest](source-manifest.json) pins 1818 source files, 42 fixtures and
three frontend files. [Build input](build/build-result.json) binds every binary,
original and candidate package, full payload inventory, CI observation, builder,
fixture, comparison log and repository file. [Handoff metadata](preparation/handoff.json)
records the exact disk paths. Large ZIP/bundle/package bytes stay in the prepared
directory; this checked-in packet retains hashes and public proof. Replaying
`load_build` requires the complete adjacent `inputs/` directory on disk.

The [raw CI observations](build/ci/job.json) retain the API's attempt labels and
original job timestamps. Earlier failed/retried AMD64 CI observations remain
in the overnight packet. No ARM64 rerun or altered observation is invented.
These are retained GitHub observations checked against its archive digest,
rather than a cryptographic build attestation. Version 1 explicitly pins the
reviewed workflow and release-container source digests.

Native compilation occurred in ARM64 CI, with release binaries built inside
Debian 12. Candidate assembly used the frozen source builder and unchanged CI
ELFs, UI and generated contracts. The new preparation command independently
reconstructs standard/shadow payloads and compares every file, mode, owner and
control, excluding archive timestamps. Actual candidate `.deb` hashes remain
fixed. The record contains no fabricated guest `apt`, `rustup`, build steps or
workspace test count. The existing native-guest validation path still requires
all its original gates.

Both signatures and the public fingerprint validate for the candidate and
previous repositories; signed indexes bind complete package bytes. The frozen
signing script streams `Release` into the directory it scans, producing a
self-entry for its already-written header. A reproduced regression now validates
that header after checking both full signatures. Complete index and package
hash checks remain mandatory. Previous artifact identity/core/hash and version
ordering pass [local checks](preparation/previous-repository.json); no actual
upgrade has occurred. The disposable key lasts two days, and the repositories
expire after seven days. Refresh test signatures before expiry without changing
the package bytes. Private signing material is excluded from this packet.

[Captured Debian tests](validation/broad-probe/debian-python311-harness.log.gz)
pass 61 cases, including the original 41 plus 20 CI/provenance regressions with
many malformed-input subcases. [Scoped checks](validation/scoped/checks.json)
pass Ruff checking/formatting on all seven changed Python files, repository
hygiene and actual offline build validation. A wider Ruff probe reports two
pre-existing `timezone.utc` aliases in untouched `native_vm.py`; its failed
[raw log](validation/broad-probe/ruff-check.log.gz) is preserved. Host guards and
deadline supervision were not changed. Raw command logs use lossless gzip; the
[evidence index](evidence-sha256.json) binds all packet files.
The two uncompressed ARM64 apt indexes use their original `Packages.gz` copies
in this packet; [compression bindings](repository-index-compression.json) retain
raw hashes and exact paragraph terminators. Complete runnable repositories stay
unchanged in the prepared disk directory.

For the next authorized window, the prepared root is:

```text
/home/marc/Documents/github/lime-os/.cache/arm64-qual/integrated-2026-10-12/ci-ready-2026-10-13
```

Use `prepared/build-result.json` together with the entire `prepared/inputs/`.
Extract `prepared/inputs/source.tar.gz` for the frozen source and guest fixtures,
and preserve the external `prepared/inputs/source-manifest.json`. Supply the
copied `prepared/inputs/repo/` as `--repo`. For genuine upgrade also copy
`previous-repo/` and `public-handoff/previous-artifact.json`. Execute the bundled
fixture scripts, preserving every result, stdout/stderr and journal before
guest teardown. The [harness instructions](../../../../tests/qualification/arm64/README.md#native-ci-input-preparation)
describe offline verification and input copying.

Obtain a new exact target/window before host access. Then use guarded preflight,
the detached deadline proof and one supervised guest at a time. Reuse the approved
dormant QEMU/image setup if still suitable. Capture current host pressure and
production-service identities; the fixed swap safeguard remains in force.
Run fresh install/footprint, genuine schema-8 upgrade and separate synthetic
storage/container interruption suites. Compilation stays off the Pi. Stop early
and retain complete guest artifacts on any safeguard or suite failure.

The [qualification status](qualification-status.json) marks every installed gate
untested. Native KVM behavior, the production kernel's 16 KiB pages, a comparable
Python baseline, production read-only shadow, migration and a tested return path
remain distinct evidence gates. Local acceptance authorizes no activation.
