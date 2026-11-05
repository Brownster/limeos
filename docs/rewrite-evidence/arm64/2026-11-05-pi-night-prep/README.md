# Pi night run: prepared off the Pi (nothing run on wybie)

**Status: prepared, not run.** No SSH connection was made, no host package was installed, and no ARM64 code was
executed. Everything below was built and checked on the workstation. The previous attempt
([2026-10-13](../2026-10-13-current-016ba49-memory-guard/README.md)) stopped at its swap guard because Rust was
compiled in a guest on the Pi; this candidate is cross-compiled locally, so no compilation happens on the Pi.

## Candidate

| Item | Identity |
| --- | --- |
| Runtime source | `df24c290c63ef8d2f4942e62ba33d2d5ef1edfe2` (main, "Engine-wide container bound"), authority schema 15 |
| Guest fixtures / harness | `1973da1bf259323e98a38129e97a809d727e59c2` (in the bundle). The orchestrator, host helper and README were refined in later commits; they run on the workstation/host only, so the bundled guest scripts are unchanged |
| Package version | `0.4.16+arm64.1` (distinct label; hashes identify the code) |
| Build | Offline cross-build, `cargo build --offline --locked --release --target aarch64-unknown-linux-gnu`, `CARGO_BUILD_JOBS=4`, image `sha256:af684f63…5ba5` (Rust 1.88.0, `aarch64-linux-gnu-gcc` 12.2.0 / glibc 2.36; binaries need at most glibc 2.34). Two separate compilations produced byte-identical binaries. [Record](inputs/cross-build-record.json), [log](inputs/cross-build-output.log.gz) |
| Frontend | Built from the archived source with `npm ci --ignore-scripts && npm test && npm run build` (offline cache); [steps](inputs/frontend-steps.json). Packaged as `ui/index.html` + `ui/assets/index-CtQPcT81.js`, `index-CJmPPZq_.css` |
| Bundle | `source.tar.gz` SHA-256 `20fc8dd6b3661cc178feed804239b8cc8c32c77f4b44c20e3e772b09a39f56f6` (57,370,480 bytes) |
| External manifest | SHA-256 `a81b0c1006a255cb295507c4a208a9ece183ff3f3b8d1b1d4b12f8ec86aa76db`; qualification kind `local-cross-build-kvm-install` |
| `build-result.json` | SHA-256 `fe4aff2b956eec6129000d68c824c4cca127c93eee27784e64e84cf241289988` (build kind `local-cross-build`) |
| Standard `.deb` | `limeos_0.4.16+arm64.1_arm64.deb` `8d8f85fff3eaaa2fc3b8d8f4381986a4f2dbcb8f50cb487eb73612f80144f26a` |
| Shadow `.deb` | `limeos-shadow_0.4.16+arm64.1_arm64.deb` `2bb18e55ccfc2bd2a9d1cbfae4d7252c446add2c8c00528feb3414231ea1ea9c` |
| Signed repository | key `BDCBF3DA53F2AAD4E2075F9A2E0261B6B14860C3` (ed25519, **expires 2026-11-12**), `InRelease` `88b8b7d7…1c4f`; all repository file hashes in [candidate-summary.json](inputs/candidate-summary.json). Private key stays in `.cache/q/signing`, uncommitted |

| Binary (aarch64, verified with `file` in both packages) | SHA-256 | Bytes |
| --- | --- | --- |
| `limeos-core` | `841594ca530f07d96c35a81be4508169de3f4f2cb9a170357077771d848ffd56` | 6,836,056 |
| `limeos-executor` | `28ec726005cbef20ba0feb5c41e00e83c45ff9e4048ea39c8ad3c8b01ecd5987` | 5,394,104 |
| `limeosctl` | `5b073d62bc4f5145961499a19f81d79bd110f02de2471bd6249207783325b28a` | 987,856 |
| `limeos-password-worker` | `a1e1371204f530c635c0693fd80c1a125a78accd53d542e94b0b48c2154841b2` | 529,024 |
| `limeos-stack-read-worker` | `e12b74835fd7317de826c085fc3d3907f9ac190247fb1e4facf6f1d0f1023013` | 1,118,928 |

On disk (not committed, too large): `.cache/arm64-qual/2026-11-05-cross/{source.tar.gz,source-manifest.json,candidate/}` in
the worktree that built it. `candidate/` holds `build-result.json` and `inputs/` (bundle, manifest, record, signed repo);
copy it whole. The Debian 12 genericcloud ARM64 image is `.cache/arm64-qual/integrated-2026-10-12/debian-12-genericcloud-arm64.qcow2`;
its SHA-512 `835b9295…4375` was recomputed and equals the value in the retained Debian
[SHA512SUMS](../2026-10-13-current-016ba49-memory-guard/inputs/debian-SHA512SUMS) of the October 13 packet.

This is weaker than a native build: no native test totals, no native lint gates, and the cross toolchain, not the
Pi, produced the ELFs. The record states that in `limitations`. Hardware behaviour is exactly what the night run tests.

## What runs in the guests

Fresh install of standard and shadow with auth, enrollment and footprint (`install_guest.py`); the container
lifecycle suite (`approved_guest.py --suite container`); the whole-stack lifecycle
(`stack_guest.py --suite lifecycle`, wrapping `p04_stack_lifecycle_guest.py`); and the stack scale case
(`stack_guest.py --suite scale`, wrapping `p04_stack_scale_guest.py`: 30, 64 and 65 containers, reader cgroup
sampling). Each is its own fresh guest, one at a time. The amd64 runs used 1024 MiB; the 64-container case in a
1536 MiB arm64 guest may be tighter than anything measured so far.

## Command

```
uv run tests/qualification/arm64/run_pi_night.py --host holly@wybie --authorization AUTH.json \
  --deadline 2026-11-06T05:30:00Z --candidate .cache/arm64-qual/2026-11-05-cross/candidate \
  --run-dir .cache/arm64-qual/night-2026-11-05 --image <image path> --execute
```

Omit `--execute` for the no-SSH check (done here: it validates the candidate offline, the image SHA-512, the
authorization window and the plan, and exits 0). `AUTH.json` is the operator's recorded window (format in the
[harness README](../../../../tests/qualification/arm64/README.md)); this preparation did not create one. The
deadline must be at least 20 minutes before the window ends. `native_vm.py` additionally stops guests 5 minutes
before the window closes.

## Guards (evaluated on the host by `pi_night_host.py`)

Refuse to start any guest: a baseline container not running; any qemu or supervisor process present; MemAvailable
below 3 GiB; swap used over 90% of swap or over 450 MiB (the previous run's absolute guard, kept); 1- or 5-minute
load over 2. The detached watchdog (5 s) stops the guest and writes `abort.json` for: a baseline container stopped,
missing, recreated or restarted; MemAvailable below 2 GiB (the previous floor); the same swap limits; a new OOM kill;
75 C; a reboot. It records timestamped events and attribution evidence and never restarts a container.

## Checks run here

All 96 harness tests pass (`python3 -m unittest discover -s tests/qualification/arm64 -p 'test_*.py'`): the 63
existing plus cross-build provenance, guard thresholds, container-stop detection, watchdog abort, one-guest-at-a-time
ordering, purge-exactly-the-added-packages and crashed-run teardown, all against fakes. `run_pi_night.py` without
`--execute` passes. Docker Desktop was started for the build and signing and stopped afterwards.

## Untested

Everything on the Pi: the SSH paths, `sudo -n apt-get` on wybie, the real QEMU install/purge round trip, scp of the
57 MB bundle and 327 MB image, the watchdog's `docker inspect` and `docker events` formats on the host, `ss -tlnp`
comparison, guest boots at 1536 MiB, all four suites on ARM64, the 1536 MiB guest with the 64-container case, and the
emulated-guest smoke (the harness has none, and none was added). Whether an unidentified client stops containers
again is not predicted: the watchdog can only record and abort. Unchanged from October: upgrade from a previous
schema is not covered (no schema-15 predecessor artifact exists), and the optional storage suite and inspector were
not scheduled.
