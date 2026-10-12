# Schema-9 Pi qualification runbook

Prepared locally on `engineer/arm64-schema9-qualification` in
`/tmp/limeos-arm64-schema9-qualification`. Exact SSH target and tonight's
Europe/London start/end times remain pending. No host has been accessed.
The October 6/7 window is expired. Record the operator's new exact target and
UTC window; do not derive permission from an alias or invent the times.

## Frozen inputs

| Input | Identity |
| --- | --- |
| Runtime/source | `1401c255d4a05b8621c2dcd497d6f7f0cbe91d56` |
| Fixture revision | `8b7ff58c5cd9cd0279e3ea73bd5904a41d15cad7` |
| Candidate package version/schema | `0.4.8+arm64.1` / 9 |
| Candidate bundle | `.cache/arm64-qual/schema9-2026-10-12/guarded/source.tar.gz` |
| Candidate manifest | Same directory, `source-manifest.json` |
| Bundle SHA-256 | `a24da0ce098cfaf74e91dced22a9640d4f0a1ac5de87b7144e70e23daca8b1a2` |
| Genuine previous source | `8acac403e32d3692b028a036a11f4fead2050580` |
| Previous package version/schema | `0.4.7+arm64.1` / 8 |
| Previous package SHA-256 | `6c86199b253e155c94d96e1395c069c4188e11a2379f14eff97eacab0203b898` |
| Previous core SHA-256 | `aab46a601a95924c8900a93749cd8cf5cc735cc2328783196732c363f258cde0` |

The previous package and provenance are in
`.cache/arm64-qual/schema9-2026-10-12/upgrade-inputs/schema8/`.
The [preparation packet](../rewrite-evidence/arm64/2026-10-12-schema9-preparation/README.md)
records complete hashes and local checks. The earlier bundle with fixtures
`7ce2d98` is retained as superseded; use only the `guarded/` bundle above.
Do not include unmerged library or recovery work in this qualification.

## Window and preflight

After receiving the operator's exact target and times, create a new local JSON
authorization containing exactly `version: 1`, `host`, `not_before`, `not_after`,
`authorized_by`, `reference` and `scope`. Use UTC `YYYY-MM-DDTHH:MM:SSZ` timestamps,
a positive window of at most 12 hours and this runbook as the reference.
Scope is disposable ARM64 KVM guests. Supply this record on every runner call;
its bounds now apply to isolated hosts as well as production hosts.

Set these variables from the supplied record and the verified Debian image:

```bash
RUN=.cache/arm64-qual/schema9-2026-10-12
BUNDLE="$RUN/guarded"
HOST=<exact-SSH-target-from-operator>
AUTH=<new-authorization-JSON>
IMAGE_SHA512=<verified-Debian-SHA512SUMS-digest>
V=(uv run tests/qualification/arm64/native_vm.py --host "$HOST" --authorization "$AUTH")
"${V[@]}" host-info --output "$BUNDLE/host-preflight.json"
```

The authorization must already be in its valid window before host access.
Check native AArch64, KVM, QEMU/AAVMF, passwordless sudo, the retained Debian 12
image, free disk/memory and production workload. Validate the image bytes against
Debian's original `SHA512SUMS`, not an unbound digest of the local file. Missing
host prerequisites require separately scoped preparation; this run does not
install host packages or change groups, services, Docker, mounts or data.

Use one guest at a time. Build ceilings are 3 CPUs/2048 MiB; test ceilings are
2 CPUs/1536 MiB. Reduce allocation under pressure. QEMU uses nice 19 and idle
I/O. No host directories or physical disks may be passed through.

## Prove the current supervisor first

Choose an earlier proof deadline at least eight minutes ahead, within the
window. Boot `s9-proof-20261012` on port 22901 with 1 CPU, 768 MiB, an 8G overlay,
the verified image digest and `--terminate-at <proof-UTC-deadline>`.
Retain its protected owner/child identity, marker, UID, kernel start ticks,
deadline and launcher hashes. Capture an unrelated existing host process's
identity before and after; never signal that process.

Let the launch SSH client end. Without calling orderly stop, observe the host
owner enforce SIGTERM at the proof deadline, SIGKILL two minutes later if
needed, and reap its QEMU child. Retain supervisor/console logs and actual exit
state. Local process mocks and the historical supervisor run are separate
evidence. Collect and stop this owned run through the normal runner before
starting the build. An unproven or failed deadline gate blocks the long run.

## Build natively

```bash
"${V[@]}" boot s9-build-20261012 --port 22902 --cpus 3 --memory 2048 \
  --disk 24G --image-sha512 "$IMAGE_SHA512"
"${V[@]}" exec s9-build-20261012 'mkdir -p /root/qual'
"${V[@]}" push s9-build-20261012 "$BUNDLE/source.tar.gz" /root/qual/source.tar.gz
"${V[@]}" exec s9-build-20261012 'cd /root/qual && tar -xzf source.tar.gz'
"${V[@]}" push s9-build-20261012 "$BUNDLE/source-manifest.json" /root/qual/source-manifest.json
"${V[@]}" exec s9-build-20261012 'python3 /root/qual/fixtures/tests/qualification/arm64/build_guest.py --work /root/qual --manifest /root/qual/source-manifest.json'
"${V[@]}" pull s9-build-20261012 /root/qual/build-result.json "$BUNDLE/build-result.json"
"${V[@]}" pull s9-build-20261012 /root/qual/logs "$BUNDLE/build-logs"
"${V[@]}" pull s9-build-20261012 /root/qual/packages "$BUNDLE/packages"
"${V[@]}" pull s9-build-20261012 /root/qual/repo "$BUNDLE/repo"
```

Record stdout/stderr, exit codes and host pressure throughout. Require Rust
1.88.0 native AArch64, all locked source gates, four AArch64 ELFs and exact
standard/shadow package controls and hashes. The expected reviewed runtime has
444 Rust cases; preserve the actual totals. Do not proceed to installed tests
with a failed or incomplete build record.

Before closing this guest, push the unchanged schema-8 standard package under
`/root/qual/previous-input/`. Use the fresh disposable signing fingerprint from
its successful `build-result.json` and frozen `source/packaging/repository.sh`
to create `/root/qual/previous-repo`. Pull that repository and retain signature
verification, fingerprint, package SHA and commands locally. This replaces the
expired test signature around the original bytes; it does not rebuild the old
package. Never disable signature or validity checks. Then collect host pressure
and stop `s9-build-20261012`.

## Fresh install and upgrade

Boot each fresh guest separately, with 2 CPUs/1536 MiB, 16G and the same image.
Extract the same guarded bundle under `/root/qual`; copy the native signed
repository to `/opt/limeos-repo`, native build record to
`/root/qual/build-result.json` and frozen `source/tests/fixtures/werkzeug-hashes.json`
to `/root/werkzeug-hashes.json`. Execute the bundled fixtures in `fixtures/`.

First use `s9-install-20261012`, port 22903. Run this command through `V exec`:

```bash
python3 /root/qual/fixtures/tests/qualification/arm64/install_guest.py \
  --repo /opt/limeos-repo --expected /root/qual/build-result.json \
  --hashes /root/werkzeug-hashes.json --output /root/qual/install-result.json
```

Allow about 20 minutes for the recorded workloads, including ten-minute idle
sampling. Retain per-service PSS/RSS/swap, CPU/write counters, readiness,
authenticated latency, installed bytes, enabled services and host load. Native
KVM results do not establish bare-metal footprint or a matched Python comparison.
Pull result, raw output and journal, then stop this guest.

Next use `s9-upgrade-20261012`, port 22904. Also copy the newly signed previous
repository to `/opt/limeos-previous-repo` and unchanged previous provenance to
`/root/qual/previous-artifact.json`. Execute:

```bash
python3 /root/qual/fixtures/tests/qualification/arm64/upgrade_guest.py \
  --repo /opt/limeos-repo --expected /root/qual/build-result.json \
  --previous-repo /opt/limeos-previous-repo \
  --previous-provenance /root/qual/previous-artifact.json \
  --output /root/qual/upgrade-schema8-result.json
```

Require actual schema 8 before package replacement and schema 9 afterwards,
preserved sessions, canonical approval bytes/token hash, verified receipts,
complete active uncertain claims and queued state. The pending approved restart
must verify, and replay must return the original job. Preserve the actual old
and installed new binary/package identities, result and journal. Stop the guest.

## Remaining time and teardown

If the window permits, use the existing `approved_guest.py --suite storage`
and `extra_guest.py` in `s9-storage-20261012` (port 22905), then
`approved_guest.py --suite container` in `s9-container-20261012` (22906).
Follow the [existing guest setup](../../tests/qualification/arm64/README.md#reproduce-a-current-run)
with these frozen inputs. Only storage gets `--storage-disks`: four empty
128 MiB qcow2 disks with existing serial/size/boot/mount guards. Preserve
interruption/recovery failures; mark omitted suites untested. Historical schema
6/7 upgrades remain supported but get no new qualification claim from an 8→9 run.

The runner refuses late launches and further dispatch after cutoff. Its host
supervisor terminates at window end minus five minutes and escalates two minutes
later. Collect and reap owned guests before the exact end. Do not retry after
expiry. Record final host pressure and unrelated process/service preservation;
retain only owned guest metadata and logs, excluding SSH private keys.

Use `record_run.py` with a new run ID, the guarded manifest, actual native build,
install and suite results, complete raw logs and host samples. Missing results
stay untested. Inventory p95 61.401 ms against 20 ms remains a historical failure.
Backup, P04 and cutover gates stay open; guest qualification does not authorize
production activation or integration of the unmerged libraries.
