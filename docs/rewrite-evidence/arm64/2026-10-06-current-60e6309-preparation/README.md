# Current ARM64 qualification preparation

Status: preparation complete; native qualification awaits an explicitly
available isolated ARM64 SSH host. This workstation is x86_64. No remote
commands or native package, upgrade, recovery or performance runs were
attempted. Hardware wait is separate from engineering work.

The assignment is
[`2026-10-06-engineer-current-arm64-qualification.md`](https://github.com/Brownster/limeos/blob/main/docs/plans/2026-10-06-engineer-current-arm64-qualification.md).
It excludes a new quiet window on wybie and Holly's production Pi. The
remaining input is the isolated host's SSH target, with native KVM and the
verified Debian cloud image described in the harness README.

## Frozen identities

| Item | Identity |
|---|---|
| Branch | `engineer/current-arm64-qualification` |
| Worktree | `/tmp/limeos-current-arm64` |
| Runtime source | `60e6309384c6a93caa63c0d578dc57d981897863` |
| Separate shared fixture adaptation | `276a88d930b345dc283727a4b01cef83f3e82403` |
| Committed harness/fixture source | `bb8676934573642b4fdc43b4df5e939d72d40c7a` |
| Planned qualification package label | `0.4.4+arm64.1`, authority schema 8, ARM64 |
| Prepared bundle | `/tmp/limeos-current-arm64/.cache/arm64-qual/current/source.tar.gz` |
| Bundle SHA-256 | `c382556f165eb80bb2f8db4e0b2a75658ad9b93b3745fb33c54c0c54e00f7a84` |

`source-manifest.json` binds 480 frozen runtime files, the independently
archived fixture files, frontend distribution files and frontend commands.
The bundle was extracted into a fresh temporary directory and every listed
file verified. The local UI was built from the frozen runtime archive;
uncommitted workstation source and pre-existing distribution files did not
enter the bundle.

## Changes and local validation

The shared fixture commit selects the actual Debian architecture and accepts
the current qualification package version. The approved storage suite can
skip its historical AMD64 artifact upgrade; the native upgrade adapter uses
a separately recorded genuine ARM64 artifact. Existing failure, serial,
disk-size and empty-mount assertions remain in place.

The harness commit replaces hardcoded package/schema values with manifest
identities, verifies native Rust 1.88 and package/binary identity, and refuses
footprint attribution after an installed identity failure. It reuses the
approved storage/container and migration helpers, adds actual apt standard
replacement/removal after the existing active-service shadow removal check,
and records optional reader/target services separately from base services
and transient password workers. Timings and ten-minute idle windows retain
raw samples. Missing counters and incomplete workloads stay explicit.

The runner requires an explicit host, checked image, guest name and recorded
host binding. Evidence collection rejects historical directories, mixed
source/build/measurement identities and overlapping artifact destinations.

Local results:

- Eight harness regressions passed, including all CLI help paths, hash/schema
  drift, failed/incomplete or mismatched builds, emulation refusal, guest
  host/name boundaries, missing counters, short idle windows and evidence
  overwrite attempts
- Ruff lint and formatting passed for the harness and modified shared fixtures
- Frozen frontend locked npm installation, eight frontend tests, TypeScript
  check and Vite production build passed using Node 22.14.0
- Bundle extraction verified all runtime/frontend/fixture hashes
- The available older ARM64 package's SHA-256 matches its historical record

Raw commands, exits and output are under `local-validation/` and `frontend/`.
`preparation-status.json` records this workstation's architecture and all
native gates as untested. The integrated native CI baseline reports 185
workspace tests; this preparation does not count those as a new installed
ARM64 run.

## Genuine older artifact

`previous-artifact.json` binds the preserved corrected native `0.4.2` package
to original source `5848bceef22961f22a00f7c599f69ba4cd6bb544`, package/core
SHA-256s and original evidence file SHA-256s. The artifact is 2,213,744 bytes,
SHA-256 `0a8651aa73afc7a2e67fe665f4c43d3a9f832e09c3104c316f105bea651c6e51`.
It is a candidate for a new schema 6 → 8 test. No new upgrade has been run.
It cannot establish schema 7 → 8 qualification. A genuine schema 7 ARM64
artifact is still needed for that separate claim.

## Remaining native work

Follow the reproducible build/install/storage/container/upgrade/evidence
commands in [the harness README](../../../../tests/qualification/arm64/README.md).
Run one disposable guest at a time, record host pressure and collect complete
logs and failures before teardown. Use the exact prepared runtime and fixture
identities for the first run. A runtime correction needs a separate minimal
commit and new qualification identity.

Native build/package gates, signed installs, installed authority/capability
checks, approved-operation crash recovery, actual teardown, genuine upgrade
preservation and all current-payload performance measurements remain
untested. The footprint fixture measures 20 containers and three dashboards
in its separate stream window, with zero registered synthetic storage disks;
the guarded storage suite uses four disks separately. The eight-disk
footprint workload and assistant-inclusive total remain untested. Native KVM,
bare-metal Pi and a comparable Python baseline are separate claims. Daily
write estimates must name the measured interval. The older 9.68 MiB result
belongs to the uncorrected historical build.
