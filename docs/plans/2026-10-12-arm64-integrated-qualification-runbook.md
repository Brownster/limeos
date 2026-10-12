# Integrated rewrite Pi qualification

This candidate supersedes the earlier schema-9 preparation for tonight's tests.
Completed before-image, SnapRAID-source, pool/protection and Samba libraries plus
the current harness are integrated. Backup-bound recovery remains excluded.
No Pi has been accessed. Exact SSH user/host and Europe/London start/end times
remain pending; do not reuse the expired October 6/7 authorization.

| Input | Identity |
| --- | --- |
| Source and fixture revision | `016ba49a27b228576db5d4ca28dcd122c60a6278` |
| Candidate package/schema | `0.4.8+arm64.2` / 9 |
| Local run directory | `/home/marc/Documents/github/lime-os/.cache/arm64-qual/integrated-2026-10-12` |
| Bundle | `source.tar.gz` in that directory |
| External manifest | `source-manifest.json` in that directory |
| Bundle SHA-256 | `fc746b8e564c015ce7bf6757ad60d833b5fd6cb88ff028c376bef3109c9414a3` |
| External manifest SHA-256 | `bee6bf0742de9b7e55fd3b9bcaeb82ac62c5cac33250360e53b88cade18cfd4c` |
| Genuine previous source/version/schema | `8acac403e32d3692b028a036a11f4fead2050580` / `0.4.7+arm64.1` / 8 |
| Previous package SHA-256 | `6c86199b253e155c94d96e1395c069c4188e11a2379f14eff97eacab0203b898` |
| Previous core SHA-256 | `aab46a601a95924c8900a93749cd8cf5cc735cc2328783196732c363f258cde0` |

The unchanged previous package/provenance is in `upgrade-inputs/schema8/` below
the new run directory. Earlier artifacts stay intact. The new bundle contains
1793 source files, 39 fixtures and three frontend-dist files, verified after
extracting on disk. Local acceptance is recorded in the
[integration packet](../rewrite-evidence/p04/2026-10-12-completed-slices/README.md).
Native build, installed behavior, crash recovery and footprint remain pending.

Follow the [schema-9 runbook](2026-10-12-arm64-schema9-qualification-runbook.md)'s
window/preflight, supervised short-deadline proof, native build, fresh install,
genuine upgrade and disposable storage/container crash sequence with these new
inputs. Use unique guest names such as `integrated-build-20261012` and
`integrated-upgrade-20261012`; do not reuse old run evidence or guest state.
The old runbook's candidate paths/versions/hashes are historical inputs only.

After the operator supplies the exact target/window, create a fresh authorization
record with precisely `version`, `host`, `not_before`, `not_after`,
`authorized_by`, `reference` and `scope`. Use version 1, exact SSH target, UTC
timestamps, a window of at most 12 hours and this runbook as the reference. Scope
is disposable ARM64 KVM guests. Supply it on every runner invocation, including
on an isolated host; it must already be valid before host access.

```bash
RUN=/home/marc/Documents/github/lime-os/.cache/arm64-qual/integrated-2026-10-12
BUNDLE="$RUN"
HOST=<exact-SSH-target>
AUTH=<new-authorization-file>
IMAGE_SHA512=<digest-from-Debian-SHA512SUMS>
V=(uv run tests/qualification/arm64/native_vm.py --host "$HOST" --authorization "$AUTH")
"${V[@]}" host-info --output "$RUN/host-preflight.json"
```

Check native AArch64, KVM/QEMU/AAVMF, sudo, the retained Debian 12 image, disk,
memory and the host's workload within the authorized window. Verify image bytes
against Debian's original SHA512SUMS. Missing host prerequisites require separate
preparation; this run installs no host package, service or group and changes no
host Docker engine, mount, filesystem or media data. An in-use Pi stays a
protected host; all active package/load/failure tests run in disposable guests.

Use one guest at a time. Build ceilings remain three CPUs/2048 MiB, test ceilings
two CPUs/1536 MiB, nice 19 and idle I/O; reduce allocation under host pressure.
Do not pass host directories or physical disks into guests. Keep local source,
build and general scratch on disk. Each guest must have the runner's host-owned
deadline supervisor. First prove termination with a short disposable deadline;
stop five minutes before cutoff and enforce forced cleanup three minutes before.
An expired window cannot authorize retry or new work.

Build using the bundled `fixtures/tests/qualification/arm64/build_guest.py` and
external manifest, not an uncommitted workstation script. Require Rust 1.88.0,
locked native source gates, four verified AArch64 ELFs, source-bound standard
and shadow packages and a fresh owned signed repository. Old repository signing
keys may be expired; sign a new guest-owned repository around unchanged genuine
previous package bytes, preserving its recorded source/package/core provenance.

Prioritize fresh install/footprint and genuine schema-8 to schema-9 upgrade.
Retain the successful build/install/upgrade results, original approvals and
canonical plan bytes/token hashes, complete uncertain claims, verified receipts,
queued state and replay decisions. Include 20-minute footprint capture when the
window permits. The current libraries' local tests are included in the native
build; they do not imply that live share/SnapRAID/restore operations are exposed.

Next run the existing synthetic storage/container crash fixtures if time remains.
Do not turn kill/restart tests into claims of real power-loss qualification.
Record every exact source/fixture/package/binary identity, check outcome and
failure. Keep untested cases explicit. New rewrite work on another branch cannot
enter this frozen candidate without another version/manifest and qualification.
