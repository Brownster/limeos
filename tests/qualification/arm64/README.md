# Native ARM64 qualification

Run LimeOS only in disposable Debian 12 ARM64 KVM guests on an explicitly
available isolated host. The workstation uses SSH through the host to a
loopback guest port. The host runs QEMU; packages, policy changes, synthetic
disks and fault injection stay inside guests. This assignment grants no new
quiet window on wybie or Holly's production Pi. `--host` has no default, and
the runner rejects production hosts without a current recorded authorization.
Only supply an explicitly available host; alternate aliases and IP addresses
are not isolation proof.

The operator subsequently authorized [one overnight window](../../../docs/plans/2026-10-06-engineer-arm64-overnight-qualification.md)
for `holly@wybie`, ending **2026-10-07 07:00 Europe/London (06:00 UTC)**.
That window has expired. The reviewed runner checks explicit expiring
authorization before every SSH/SCP dispatch and uses a host-owned guest
launcher. Default wybie rejection applies outside a current window; historical
authorization grants no new host work. Preserve the historical input bundles
and [native run evidence](../../../docs/rewrite-evidence/arm64/2026-10-07-current-8acac40-overnight/README.md),
including its recorded temporary host packages and remaining limits.

The original engineer preparation freezes runtime source at
`60e6309384c6a93caa63c0d578dc57d981897863`, authority schema 8. Use a distinct
qualification package version, for example `0.4.4+arm64.1`. Package labels
identify an artifact; the source, fixture, binary and package hashes identify
the code tested. Earlier evidence stays under its original directory.

The integrated preparation pins runtime and fixtures to
`8acac403e32d3692b028a036a11f4fead2050580`, schema 8, version
`0.4.7+arm64.1`. Its bundle is under
`.cache/arm64-qual/2026-10-06-integration/`; the [integration evidence](../../../docs/rewrite-evidence/p04/2026-10-06-handoff-integration/README.md)
records its digest. The [overnight report](../../../docs/rewrite-evidence/arm64/2026-10-07-current-8acac40-overnight/README.md)
records native build/install/footprint and genuine upgrades for that runtime
with separately committed build-fixture fixes and a rebuilt bundle. The later
[integration record](../../../docs/rewrite-evidence/p04/2026-10-07-handoff-integration/README.md)
keeps corrected runner proof separate from that historical native run.
Supply exact committed identities when reproducing a preparation; preserve
the older bundles too.

## Inputs and scripts

| Script | Location | Purpose |
|---|---|---|
| `native_vm.py` | Workstation | Explicit host, checked image, bounded guest resources, synthetic disks, platform/pressure capture, SSH copy/exec and teardown |
| `make_bundle.py` | Workstation | Archive frozen runtime and separate fixture commits; build/test UI against archived source; hash all inputs |
| `build_guest.py` | Native build guest | Verify inputs and Rust 1.88.0 native toolchain; locked tests, strict Clippy, contracts, dependency checks, release binaries, standard/shadow packages and signed repository |
| `install_guest.py` | Fresh guest | Signed install, schema/identity, dormant units, ownership/capabilities, imported hashes, bounded workers, durable sessions, shadow refusals and footprint |
| `approved_guest.py` | Fresh storage/container guests | Reuse current approved-operation suites with original assertions; actual apt replacement/removal on storage guest |
| `upgrade_guest.py` | Fresh upgrade guest | Genuine recorded schema 6/7 artifact upgrade, sessions, pending approval, receipts, active locks/claims and replay |
| `extra_guest.py` | Storage guest after approved suite | Exact installed hashes, command timings and separate optional reader/target memory/capabilities |
| `guest_supervisor.py` | KVM host, detached root-owned launcher | Start foreground QEMU, retain its child/pidfd, verify exact process identities and enforce shutdown/cleanup independently of workstation SSH |
| `inspector_guest.py` | Fresh native build guest | Build the frozen archive inspector example natively and record its own `VmHWM` over synthetic archives (standalone RSS, not service PSS) |
| `record_run.py` | Workstation | Reject mixed identities/overwrites; collect raw artifacts, summaries, budget comparisons and evidence hashes |
| `test_harness.py` | Workstation | Local regressions without SSH or installed-service mutations |

Guest scripts use Debian's Python 3.11 stdlib; workstation scripts use `uv`.
All scripts support `--help`. Exit 0 means checks passed, 1 means a failed
runtime gate, and argparse uses 2 for invalid usage. Save command stdout/stderr
and exit codes, including failures. A failed installed identity refuses
footprint measurement. A failed suite is retained in its result JSON.

Commit small shared fixture adaptations separately. The bundle's
`--fixtures-commit` includes both `tests/qualification/arm64/` and
`tests/privileged_vm/`; guest wrappers verify these bytes independently of
`--commit`. Execute the bundled fixture scripts, not the historical scripts
in `source/tests/` or an uncommitted workstation copy.

## Production host authorization and guest deadlines

`--host` still rejects production hosts by default: `wybie` by name, and any
alias or address that resolves to it (`ssh -G` without connecting, then
address comparison). A production host is usable only with
`--authorization FILE`, a recorded JSON window with exactly `version` (1),
`host` (the exact `user@host` string passed to `--host`), `not_before` and
`not_after` (UTC, `YYYY-MM-DDTHH:MM:SSZ`, at most 12 hours apart),
`authorized_by`, `reference` and `scope`. Absent, malformed, mismatched,
not-yet-valid or expired authorization is refused before dispatch. Every host
and guest SSH/SCP entry point re-checks the window and limits its local client
timeout to the remaining interval minus one second, preserving a stricter
caller timeout. The authorization's SHA-256 is stored in each guest's state.
Stopping an SSH client does not terminate arbitrary work already dispatched
on the host; generic remote background commands have no lifetime guarantee
from this transport limit.

Every guest has a separate host-side owner and deadline. `boot` copies the
launcher and command into a private root-owned directory and verifies their
hashes. A detached root-owned launcher starts foreground QEMU itself, using
a cleared environment and isolated Python imports. It retains the child and
pidfd before publishing its identities; bind/state/log failures trigger
owned-child termination and reaping. It never relies on another workstation
SSH round trip to establish guest supervision.

The owner sends SIGTERM at the deadline and SIGKILL two minutes later. For
an authorized host those times are `not_after` minus 5 and 3 minutes; `boot`
refuses when less than 15 minutes remain. `--terminate-at` can set an earlier
proof deadline. Orderly stop validates start time, UID and the exact `-name`
marker before signalling through a pidfd; there is no numeric-PID fallback.
It waits for the owner before removing the protected control directory.

The corrected launch path has local process/mock proof, including loss of
its simulated SSH parent before identity publication; current-source CI is
reported separately in the integration record. It has not run on a new native
host. The historical overnight deadline proof used the earlier supervisor;
preserve that distinction when planning another isolated run.

## Host prerequisites

An explicitly available native ARM64 host needs KVM, passwordless sudo,
`qemu-system-arm qemu-efi-aarch64 qemu-utils` installed without recommends,
and `~/limeos-arm64-qual/image/debian-12-genericcloud-arm64.qcow2`. Verify the
image against Debian's `SHA512SUMS`; pass the full verified SHA-512 to every
boot. The runner checks native host architecture, `/dev/kvm` and image bytes
before creating a guest. The workstation needs SSH/scp, `genisoimage`, Node
and npm compatible with the frozen frontend, and `uv`.

Build guests use at most 3 CPUs and 2048 MiB; test guests use 2 CPUs and
1536 MiB. Run one guest at a time and check host pressure before each boot.
The earlier 2.5 GiB guest forced host services into swap. Resource ceilings
are maxima, not evidence that a busy host has sufficient free memory. QEMU
uses native KVM, nice 19 and idle I/O priority, opening KVM as root then
dropping to the SSH account. Capture host pressure during long runs with
`host-info`; keep host/build load separate from installed-service results.

## Reproduce a current run

In the qualification worktree, after committing the harness:

```bash
uv run tests/qualification/arm64/test_harness.py -v
uv run tests/qualification/arm64/make_bundle.py \
  --commit 60e6309384c6a93caa63c0d578dc57d981897863 \
  --fixtures-commit <committed-harness-revision> \
  --package-version 0.4.4+arm64.1 --authority-schema 8 \
  --output .cache/arm64-qual/current/source.tar.gz \
  --manifest .cache/arm64-qual/current/source-manifest.json

# Substitute the explicitly available test host and verified image digest.
V=(uv run tests/qualification/arm64/native_vm.py --host user@isolated-arm64)
IMAGE_SHA512=<full-verified-Debian-digest>
"${V[@]}" host-info --output .cache/arm64-qual/current/host-preflight.json
"${V[@]}" boot current-build --port 22801 --cpus 3 --memory 2048 --disk 24G \
  --image-sha512 "$IMAGE_SHA512"
"${V[@]}" exec current-build 'mkdir -p /root/qual'
"${V[@]}" push current-build .cache/arm64-qual/current/source.tar.gz /root/qual/source.tar.gz
"${V[@]}" exec current-build 'cd /root/qual && tar -xzf source.tar.gz'
# Replace the internal manifest with the external copy that includes bundle hash.
"${V[@]}" push current-build .cache/arm64-qual/current/source-manifest.json /root/qual/source-manifest.json
"${V[@]}" exec current-build 'python3 /root/qual/fixtures/tests/qualification/arm64/build_guest.py --work /root/qual --manifest /root/qual/source-manifest.json'
"${V[@]}" pull current-build /root/qual/build-result.json .cache/arm64-qual/current/build-result.json
"${V[@]}" pull current-build /root/qual/logs .cache/arm64-qual/current/logs
"${V[@]}" pull current-build /root/qual/packages .cache/arm64-qual/current/packages
"${V[@]}" pull current-build /root/qual/repo .cache/arm64-qual/current/repo
"${V[@]}" host-info --output .cache/arm64-qual/current/host-after-build.json
"${V[@]}" stop current-build
```

Create fresh guests named `current-install`, `current-storage`,
`current-container`, and `current-upgrade` in sequence (ports 22802–22805).
Use 1536 MiB, 2 CPUs, 16G and the same image digest. Only the storage guest
needs `--storage-disks`: four empty 128 MiB qcow2 devices with fixed serials
`limeos-test-0` through `limeos-test-3`. The reused storage suite validates
every serial, size and absence of mounts before partition/format operations.

For each guest, copy/extract the same bundle under `/root/qual`, copy the
native signed repository to `/opt/limeos-repo`, native `build-result.json`
to `/root/qual/build-result.json`, and frozen
`source/tests/fixtures/werkzeug-hashes.json` to `/root/werkzeug-hashes.json`.
Copy the external manifest when retaining source-manifest bytes. Execute:

```bash
# Fresh install guest: allow about 20 minutes for the recorded workload/windows.
python3 /root/qual/fixtures/tests/qualification/arm64/install_guest.py \
  --repo /opt/limeos-repo --expected /root/qual/build-result.json \
  --hashes /root/werkzeug-hashes.json --output /root/qual/install-result.json

# Fresh storage guest, with the guarded synthetic disks.
python3 /root/qual/fixtures/tests/qualification/arm64/approved_guest.py \
  --suite storage --repo /opt/limeos-repo --expected /root/qual/build-result.json \
  --hashes /root/werkzeug-hashes.json --output /root/qual/storage-result.json
python3 /root/qual/fixtures/tests/qualification/arm64/extra_guest.py \
  --expected /root/qual/build-result.json --output /root/qual/extra-result.json

# Separate fresh container guest.
python3 /root/qual/fixtures/tests/qualification/arm64/approved_guest.py \
  --suite container --repo /opt/limeos-repo --expected /root/qual/build-result.json \
  --hashes /root/werkzeug-hashes.json --output /root/qual/container-result.json
```

For upgrade, copy a signed repository containing the exact earlier ARM64
artifact to `/opt/limeos-previous-repo`. If the historical disposable key has
expired, sign a new test repository around those unchanged package bytes
using `source/packaging/repository.sh`; retain the new signing fingerprint.
Do not rebuild or relabel the artifact. The previous provenance JSON needs
`source_commit`, `package_version`, `architecture`, `authority_schema`,
`package_sha256`, `core_sha256`, original evidence paths and their SHA-256s.
The corrected historical `0.4.2` artifact is available for schema 6 → 8;
its provenance is in the current preparation evidence directory. A schema
7 → 8 claim additionally requires an original schema 7 ARM64 artifact.

The integrator verified a genuine schema 7 candidate in
`docs/rewrite-evidence/p04/2026-10-06-handoff-integration/previous-schema7-artifact.json`:
original source `f39396b`, package `dist/ci-37386421570-arm64/limeos_0.4.3_arm64.deb`.
Run the schema 6 and schema 7 upgrades in separate fresh guests, with each
candidate's exact provenance and package bytes. Both integrated upgrades
pass for frozen runtime `8acac40` in the overnight report. A later runtime or
hardened host launcher still requires its own source-pinned qualification.

```bash
python3 /root/qual/fixtures/tests/qualification/arm64/upgrade_guest.py \
  --repo /opt/limeos-repo --expected /root/qual/build-result.json \
  --previous-repo /opt/limeos-previous-repo \
  --previous-provenance /root/qual/previous-artifact.json \
  --output /root/qual/upgrade-result.json
```

Pull each result, fixture result, complete stdout/stderr, journal and host
samples before stopping its guest. `stop` retains console/metadata locally
under `.cache/arm64-qual/stopped/<guest>/`. A failure is evidence to retain;
record the attempted command, exit and untested subsequent gates. Runtime
fixes require an independent minimal commit and a new qualification identity.

```bash
uv run tests/qualification/arm64/record_run.py \
  --run 2026-10-06-current-60e6309-native-1 \
  --source-manifest .cache/arm64-qual/current/source-manifest.json \
  --build-result .cache/arm64-qual/current/build-result.json \
  --install-result .cache/arm64-qual/current/install-result.json \
  --extra-result .cache/arm64-qual/current/extra-result.json \
  --suite-result .cache/arm64-qual/current/storage-result.json \
  --suite-result .cache/arm64-qual/current/container-result.json \
  --suite-result .cache/arm64-qual/current/upgrade-result.json \
  --artifact .cache/arm64-qual/current/logs=build/logs \
  --artifact .cache/arm64-qual/current/host-preflight.json=host/preflight.json
```

## Measurement limits

Idle CPU uses at least 600 seconds. Store per-service PSS/RSS/swap, raw memory
and counter samples, actual enabled services, CPU/kernel/page size/RAM/storage
and load. Timings retain individual samples. The footprint workload contains
20 static busybox containers, three dashboard streams in their separate
window and zero synthetic registered storage disks; the storage suite uses
four disks separately. The architecture's eight-disk footprint workload
and assistant-inclusive total remain separate untested gates.

Reader/target services use separate samples; password workers are transient.
Docker/containerd/workload memory is reported separately. Resident write
MB/day is an estimate extrapolated from the stated interval. Missing process
counters produce an unavailable result. Native CI, native KVM, bare-metal Pi
and a comparable Python baseline are separate claims. Never attribute the
old uncorrected build's 9.68 MiB PSS to the current or corrected payload.

Historical command interfaces and evidence remain reproducible from the
historical source/fixture commits recorded in those runs' manifests. The
current explicit-argument interface replaces their hardcoded version/schema.
