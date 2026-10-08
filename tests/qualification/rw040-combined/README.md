# RW-040 combined disposable-guest qualification

This harness prepares the qualification of production-root Engine, process,
host-source and filesystem collection, the confinement refusals and resource
admission in an owned, disposable Debian 12 AMD64 KVM guest. **Positive combined
results need the integrator's exact probe**
(`bins/executor/tests/combined_read_probe.rs`), which the base `2fd7420` doesn't
contain. Until it is supplied, every probe case is reported as `blocked`, never
replaced by a weaker substitute. The brief is
`docs/plans/2026-10-08-engineer-rw040-confinement-qualification.md` on main.

## Run

```sh
# Inputs: the current Debian 12 amd64 cloud image with Debian's SHA512SUMS,
# and the exact-source packages (CI run 37746766303 for 2fd7420).
uv run tests/qualification/rw040-combined/run_guest.py \
  --packages-dir .cache/rw040/ci-37746766303 --output FRESH_DIR
uv run tests/qualification/rw040-combined/run_guest.py --sweep   # after a crash
python3 -m unittest discover -s tests/qualification/rw040-combined -p 'test_*.py'
```

Defaults are 2 vCPUs, 1,536 MiB, a 1,800-second hard deadline and a 65-consumer
membership fixture. The image is verified against `SHA512SUMS`. Packages are
verified against `tests/fixtures/rw040-combined/packages-2fd7420.json`, or, with
`--derive-packages`, recorded from packages built in the same workflow. The guest
verifies the package hashes again and checks the installed binaries against them.

## Transport and ownership

| Concern | Mechanism |
|---|---|
| Inputs | A read-only raw tar disk (serial `rw040-inputs`): the guest programs, fixtures, verified packages and a manifest. Its digest is recorded. |
| Start | A NoCloud seed with no accounts, keys or passwords. SSH is masked at boot, and `runcmd` starts one root program. |
| Results | The guest writes one tar stream to a raw results disk (serial `rw040-results`) and powers off. The host copies only bounded regular files with safe names and requires `done.json`. |
| Console | The serial console is captured to `console.log`; `[rw040 …]` progress lines are echoed. |
| Control | A QMP Unix socket for `query-status` and `quit`. There is no other monitor. |
| Network | User-mode networking for guest `apt` only. No host forwards. `-virtfs`, `-fsdev`, `-chardev`, `tcp:`, `telnet:` and SSH are refused before launch. |
| Ownership | QEMU is a direct child, named with a unique `rw040-qual-<run>` marker, with parent-death SIGKILL. `owner.json` records the PIDs and kernel start ticks. |
| Deadline | On expiry: QMP `quit`, then SIGKILL, then reap. The guest also skips stages past its own budget. |
| Teardown | The run directory is always removed. `run.json` records its removal and any surviving marker process. `--sweep` kills only a QEMU whose PID, start ticks and marker all match, and keeps runs whose harness is alive. |

Everything the guest creates (packages, the Docker daemon, containers, users,
tasks, disks and mounts) exists only inside the guest. The workstation's Docker,
any production host, SSH, the Pi and wybie are never used.

## What a run records

`run.json` (host) binds the source commit, the dirty flag, harness and fixture
hashes, the image SHA-512, package hashes, the inputs-disk digest, QEMU argv and
version, the QMP status, the deadline outcome and teardown. The guest stages
write:

| File | Content |
|---|---|
| `platform.json` | Kernel, page size, CPU, systemd, Yama and `suid_dumpable` state, and the cloud-init schema check |
| `packages.json`, `service.json` | Verified inputs; installed versions; the installed reader unit's hash, binaries and confinement settings |
| `confinement-equivalence.json` | A transient unit with the unit's own `[Service]` settings, compared property-by-property with the installed `limeos-storage-reader.service`, plus sandbox defaults. The run fails on any difference. |
| `ground-truth/` | Independent facts from the Engine API (`/run/docker.sock`), socket activation and `SO_PEERCRED`, `/proc`, `findmnt`, `lsblk` and `blkid`, for empty and complete membership |
| `environment/baseline.json` | What unrestricted root and the reproduced confinement may do: `openat2`, `statx`, pidfd, Docker socket and peer credentials, `AF_INET`, and per-target `/proc` namespace and root access. `diagnostic_*` variants each remove one existing setting to attribute a refusal; they grant nothing and are never qualification modes. |
| `selfcheck/ceilings.json` | A harness workload (not a probe) proving that the descriptor, task and memory ceilings are enforced, with external sampling of descriptors, RSS, `VmHWM`, PSS, cgroup memory and closure |
| `transitions/verification.json` | Owned same-PID root and namespace switches, restart, exit, a swapped device under a fixed path, and a Btrfs consumer, each checked with independent facts |
| `budget/feasibility.json` | Membership scaled to 65 consumers, with guest memory before and after |
| `cases.json` | Every acceptance case: blocked with its missing prerequisite and preparing stages, or run |

## Proposed CI invocation for integration

The harness doesn't edit workflows. A shared-workflow step the integrator could
add after the package build, mirroring the existing `debian-vm` job:

```yaml
- name: RW-040 combined guest qualification (console/QMP, no SSH)
  run: |
    sudo setfacl -m "u:$(id -un):rw" /dev/kvm
    mkdir -p .cache/rw040/image
    curl --fail --location -o .cache/rw040/image/SHA512SUMS https://cloud.debian.org/images/cloud/bookworm/latest/SHA512SUMS
    curl --fail --location -o .cache/rw040/image/debian-12-genericcloud-amd64.qcow2 https://cloud.debian.org/images/cloud/bookworm/latest/debian-12-genericcloud-amd64.qcow2
    uv run tests/qualification/rw040-combined/run_guest.py --packages-dir dist --derive-packages \
      --install-package "$(basename dist/limeos_*_amd64.deb | grep -v '+ci')" --output rw040-guest-result
```

The step needs `qemu-system-x86`, `qemu-utils`, `genisoimage` and Python with
`uv`. `--derive-packages` reads `data.tar.zst` with Python 3.14, or falls back to
`dpkg-deb`. Until the probe exists, a passing run means "fixtures,
equivalence, baselines and self-check verified; probe cases blocked", not
RW-040 success.

## Limits

The guest needs about 1.1 GiB of RAM at 65 running consumers. Measurements are
from a KVM guest with 4 KiB pages under the guest kernel, not from bare metal,
a Pi or an installed combined worker. ARM64 is out of scope.
