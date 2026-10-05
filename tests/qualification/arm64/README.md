# Native ARM64 qualification

These scripts build, install and measure LimeOS on real ARM64 hardware. Every LimeOS process runs inside a disposable Debian 12 arm64 KVM guest. The Raspberry Pi only hosts QEMU, and nothing from LimeOS is installed on it. The workstation drives everything over SSH, using the Pi as a jump host to reach the guest's loopback-forwarded SSH port.

| Script | Runs on | Purpose |
|---|---|---|
| `native_vm.py` | workstation | Boot, reach, copy to/from and stop a guest on the Pi |
| `make_bundle.py` | workstation | `git archive` of the frozen commit plus built `frontend/dist`, with a SHA-256 manifest |
| `build_guest.py` | build guest | Rust 1.88 via rustup, `cargo fetch --locked`, fmt, strict Clippy, tests, contract check, release build, standard/shadow `.deb` packages, signed test repository |
| `install_guest.py` | clean guest | Install from the signed repository; identity, units, accounts, capabilities, ownership, Docker access, enrollment, imported hashes, password worker, session durability, shadow ceiling; memory, CPU, startup, latency and bytes written |
| `upgrade_guest.py` | clean guest | Upgrade from an exact earlier ARM64 artifact (hash-checked) to the native build |

The P04 storage-target suite runs unchanged from `tests/privileged_vm/` on a guest with four empty 128 MiB disks (`--storage-disks`). Its only ARM64 change is reading the package architecture from `dpkg`.

## Host requirements

- An ARM64 machine with KVM (for example a Pi 5), passwordless `sudo` and these packages: `qemu-system-arm qemu-efi-aarch64 qemu-utils` (install with `--no-install-recommends`).
- Under `~/limeos-arm64-qual/image/`, the Debian cloud image `debian-12-genericcloud-arm64.qcow2`, checked against Debian's `SHA512SUMS`.
- QEMU starts with `sudo` so it can open `/dev/kvm`, then drops to the SSH user with `-runas`. No group or ACL changes are needed. The guest runs at `nice 19` and idle I/O priority.

Keep the guest's memory well below the host's available memory. A 2.5 GiB build guest pushed about 190 MiB of the Pi's services into swap; 1.5 GiB guests did not.

## Reproduce

```bash
# Workstation, in a worktree at the frozen commit
(cd frontend && npm ci --ignore-scripts && npm test && npm run build)
uv run tests/qualification/arm64/make_bundle.py --commit <commit> \
  --output .cache/arm64-qual/bundle/source.tar.gz --manifest .cache/arm64-qual/bundle/source-manifest.json

V="uv run tests/qualification/arm64/native_vm.py --host <user>@<pi>"
$V boot build --port 22801 --cpus 3 --memory 2048 --disk 24G
$V exec build 'mkdir -p /root/qual'
$V push build .cache/arm64-qual/bundle/source.tar.gz /root/qual/source.tar.gz
$V push build tests/qualification/arm64/build_guest.py /root/qual/build_guest.py
$V exec build 'cd /root/qual && tar -xzf source.tar.gz && python3 build_guest.py'
$V exec build 'cd /root/qual/source && PATH=/root/.cargo/bin:$PATH cargo test --workspace --locked --no-fail-fast'
# Pull /root/qual/{repo,packages,logs,build-result.json} and target/release binaries, then:
$V stop build

$V boot install --port 22802 --cpus 2 --memory 1536 --disk 16G
$V push install <repo> /opt/limeos-repo        # plus expected.json, tests/fixtures/werkzeug-hashes.json, install_guest.py
$V exec install 'cd /root/qual && python3 install_guest.py /opt/limeos-repo expected.json werkzeug-hashes.json install-result.json'
$V stop install

$V boot upgrade --port 22803 --cpus 2 --memory 1536 --disk 16G
$V exec upgrade 'python3 /root/qual/upgrade_guest.py OLD.deb OLD_SHA256 /opt/limeos-repo expected.json result.json'

$V boot storage --port 22804 --cpus 2 --memory 1536 --disk 16G --storage-disks
# copy p04_targets_guest.py, p04_planning_guest.py, p04_storage_guest.py and werkzeug-hashes.json to /root
$V exec storage 'cd /root && python3 guest.py /opt/limeos-repo /root/result.json'   # guest.py = p04_targets_guest.py
```

`expected.json` holds the build's binary and package SHA-256 values; the install and upgrade guests refuse binaries that differ. Each run's evidence goes in its own directory under `docs/rewrite-evidence/arm64/`.

## Limits

- Results come from a KVM guest on a Cortex-A76 (Pi 5), not bare metal. The guest kernel uses 4 KiB pages; the Pi's own kernel uses 16 KiB pages, which raises RSS and PSS for the same binary.
- The Pi's own services keep running beside the guest. Host load is recorded with each run.
- Docker inside the guest uses Debian's `docker.io` with a static busybox workload, not real media applications.
