# Exact 46f7 U0/B0 input review

Source: `46f7daadcc5d930ca97925d96f05baddb20e1b2f`. CI run: `38076433426`. This record is separate from failed source 6964/run 38075054820. Review is read-only; no artifact collection, repeated source gates, guest launch or host changes by this reviewer.

## Verifier source review

Reviewed `/home/marc/Documents/github/lime-os/target/rw040-u0-b0-46f7/verify_actual_inputs.py`, SHA-256 `5914ecd3b78570d685361a7294f1ae18390cb6eed209380b7dcfd8c76ef02fe4` (10,845 bytes), as a finite owned input-proof helper. It is not new project runtime code. Run it using ordinary unoptimized Python so its assertions execute.

The verifier requires the exact clean detached Git head and the sole collector's same-source, successful native AMD64 job/release step. For each of the three fixed artifact names it compares saved ZIP bytes with the API digest, rejects unexpected/duplicate/unsafe ZIP members, and compares extracted regular-file bytes with those members. The probe uses the already-reviewed consumer's closed eight-field manifest and fixed public probe source checksum.

It independently checks a unique Cargo integration-test artifact's source/manifest/release profile/executable path and genuine terminal build success; the initial comparison with `[True]` accepted JSON numeric `1`, and the writer corrected it to exactly one terminal record with `success is True`. The producer source already had the correct check and remains unchanged.

It reads the actual executable's ELF identity and program interpreter, recalculates dynamic libraries and required GLIBC versions, checks Rust 1.88.0/native AMD64 and glibc 2.36 provenance, and binds the resolved builder image identity. It compares every producer-listed source input and the lockfile with exact current Git bytes; compares all three package hashes/controls with recorded provenance and checks matching installed binary maps. Finally it verifies the copied Debian image against the preserved checksum file and records only the owned U0/B0 deadline/resource/teardown scope.

Source review: **GO for using this corrected verifier to inspect actual inputs**. This is not input or guest execution GO. Inspect the collector's raw API snapshot, ZIP/API hashes, actual native compiler/build provenance and source-bound package/probe records when present; no saved manifest alone proves those origins. Recalculated `readelf` output is proof for the actual executable, independently of supplied readelf text. Keep the existing owned cleanup/deadline review and unchanged-unit policy boundary.

## Actual inputs

At the last inspected sole-collector snapshot, `status-20261010T184418Z.json`, both native jobs were building release artifacts and no artifacts existed. No acquisition/verified-inputs record, qualifying AMD64 ZIP/package/probe supply, or guest result existed. Actual input verdict remains **PENDING / NO LAUNCH**.

Authorized eventual scope is only U0 guest-root control and B0 exact unchanged installed-reader standalone library/tool baselines. All 17 combined acceptance cases remain blocked; no new profile, worker/headroom qualification, production policy change, P04 gate or cutover claim follows from these inputs.

## Actual supply received and reviewed

The sole collector subsequently supplied `input-acquisition.json` (SHA-256 `630acc10de74a0a5fbc896d9e34a37f2a9136302fba1a3b409f89126e391cf6a`) and raw API snapshot `status-20261010T184810Z.json`. The actual native AMD64 job `114284257837` completed successfully at 18:46:46Z. Its exact job object matches the saved raw API response. Release/probe production, the subsequently ordered browser/rejection gates, and all three uploads completed successfully. The source remains the exact clean detached 46f7 head.

Independently read and hashed every raw ZIP, matched each against both acquisition and raw API digests, and compared every extracted member byte with its ZIP member:

| Artifact ID / name | Actual ZIP SHA-256 |
|---|---|
| 11679172140 / debian-ubuntu-24.04 | `254828429951ddda30f87cf9289beae96d9dd24479f1c961466337336cafeb19` |
| 11679461809 / rw040-public-library-probe | `cdbc30aab723652cc1a4aa536479ee4df3590d33a54ca9e6c5639b65cd1fe557` |
| 11679391900 / rw040-probe-build-evidence | `767cd1de83346d1f87f98d82238b8bd8d4f1ad93a8853b146c0da6c966a0ef80` |

All API origins bind source 46f7 and run 38076433426. Fixed member sets are respectively three package files, three probe files and seven build-evidence files. No artifact was downloaded or extracted by this reviewer.

The actual Cargo stream contains one selected integration-test artifact, `/build/target/release/deps/combined_read_probe-5af8ab6f36dee3b6`, with the exact executor manifest/probe source, release opt-level 3, test=true and debug assertions=false. It ends with exactly one JSON boolean `build-finished.success=true`. The supplied executable is 2,151,264 bytes and hashes to `523f6e94573e975c6f2410bc641573d4e836f0bf62a1a5a90559e66271de9268`; its public probe source retains fixed SHA `3929400eb457937923311206201e4ef8ce2b935afc685ec7e81b27e557699b9c`. The closed manifest, build provenance and real native log agree on this identity.

Read actual ELF64/little-endian/AMD64 header and the verifier's recalculated `readelf` outputs: expected `/lib64/ld-linux-x86-64.so.2`, only libc/libgcc_s/libm, maximum required GLIBC 2.34. Native Rust is 1.88.0, build libc 2.36; the resolved Linux/AMD64 builder image is `sha256:44a5a83e14ce3c4764a0299384fcf675ddedb004415e9ae87e9384e871823550`. Its empty RepoDigests reflects the locally built image, whose content-addressed ID is recorded; the actual native log also preserves its resolved bookworm base digest.

Independently matched all 151 source-input byte counts and SHA-256 values against the exact clean worktree. The three package hashes/AMD64 controls in provenance match acquired files and the corrected verifier's successful package inspection. The derived manifest shows identical core, executor, password-worker and CLI binary maps across release, upgrade and shadow packages. This does not imply the standalone test executable is the installed executor binary; they are distinct products of the same exact-source workflow.

The corrected verifier was actually executed with `env -u PYTHONOPTIMIZE python3 ...` and exit 0. Its `verified-inputs.json` SHA is `d3b417ebf01f82ff993a3629e197567c5ecaea288c0e76bfee573892c2d92f02`; `input-verification-command.json` SHA is `69a07e68ad94832d3ff61b66b8c5d1cdb625fa366edb89db995d3ea08f0e6785`. It reverified the previously reviewed copied image SHA-512 `a09170d17e13af43da61774666f520e3bbec0f8bd96b4db37c15f3426fb327ccad950b80dbe78c09aa5ca90ce5015e99334b0ba5188fd4018736bc3fb5f0882c` against checksum-file SHA-256 `ae259f900fcfde80f581790a26befe14977c94246442e8a9c3e290b921d7b550`.

**Actual-input GO for the already authorized finite U0/B0 disposable guest only**, subject to the existing owned launch/deadline/teardown preflight: 2 vCPU, 1,536 MiB, 1,800-second runner deadline, 1,680-second guest budget, 65-consumer bound and work root `/home/marc/Documents/github/lime-os/target/rw040-guests`. No policy variants, production units/capabilities, workstation workloads or external host access. At review, guest_launched=false. Results and teardown require later review; all 17 combined cases and P04/BKP/cutover gates remain open.

## Initialization attempt 01

Read the writer's `attempt-01/run.json`, command capture and stderr. The exact reviewed source/probe/package/image identities are recorded. QEMU was invoked and the initial QMP Unix socket connect immediately raised sandbox `PermissionError: EPERM`. The run outcome is failed, its owned directory was removed, and the saved marker-process scan is empty. No library result or qualification was produced. Initialization raised before serial/QEMU logs were copied, so those records are absent; do not infer that a VM process was never invoked.

The unchanged authorized arguments may be retried in a fresh attempt under the tool's required permission mechanism. At subsequent read-only checks, neither the proof output nor the configured owned work root contained attempt02/status/owner/serial/stage files; the work root existed and was empty. This establishes no published retry capture at that observation, not the tool approval state. No retry result is claimed. The attempt01 wrapper command record still said running/exit=null when inspected; the writer was asked to finalize that stale wrapper record against the already preserved failure.
