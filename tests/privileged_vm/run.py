#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Boot a disposable Debian 12 KVM guest and run package/security failure tests.

Requires qemu-system-x86_64, qemu-img, genisoimage, ssh, scp and /dev/kvm.
Image SHA512 is verified against Debian's HTTPS-published checksum list.
Only a throwaway SSH key is accepted; no passwords, host mounts or host services.
"""

import argparse
import hashlib
import json
import re
import shlex
import socket
import subprocess
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def run(*args: str, **kwargs):
    return subprocess.run(args, check=True, **kwargs)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repository", type=Path, required=True)
    parser.add_argument(
        "--previous-repository",
        type=Path,
        help="Previously qualified packages for an upgrade test without rebuilding the candidate",
    )
    parser.add_argument(
        "--build-bundle",
        type=Path,
        help="Optional trusted local source/toolchain bundle; build only inside the throwaway VM",
    )
    parser.add_argument(
        "--build-output",
        type=Path,
        help="Copy freshly built Debian binaries and packages here",
    )
    parser.add_argument(
        "--package-version",
        default="0.3.2",
        help="Version label for an optional fresh build",
    )
    parser.add_argument(
        "--retain-previous-repository",
        action="store_true",
        help="Keep the input repository in the guest for a real package upgrade test",
    )
    parser.add_argument(
        "--guest-script", type=Path, default=ROOT / "tests/privileged_vm/guest.py"
    )
    parser.add_argument(
        "--storage-disks",
        action="store_true",
        help="Attach four empty 128 MiB disposable virtual disks for storage acceptance",
    )
    parser.add_argument(
        "--image",
        type=Path,
        default=ROOT / ".cache/p01-vm/debian-12-genericcloud-amd64.qcow2",
    )
    parser.add_argument(
        "--checksums", type=Path, default=ROOT / ".cache/p01-vm/SHA512SUMS"
    )
    parser.add_argument(
        "--output", type=Path, default=ROOT / "docs/rewrite-evidence/p01/vm-result.json"
    )
    args = parser.parse_args()
    if not re.fullmatch(r"[0-9][A-Za-z0-9.+~:-]*", args.package_version):
        parser.error("Invalid Debian package version")
    if args.previous_repository and args.retain_previous_repository:
        parser.error("Choose --previous-repository or --retain-previous-repository")
    if args.retain_previous_repository and not args.build_bundle:
        parser.error("--retain-previous-repository requires --build-bundle")
    expected = next(
        line.split()[0]
        for line in args.checksums.read_text().splitlines()
        if line.split()[-1].lstrip("*") == args.image.name
    )
    with args.image.open("rb") as stream:
        actual = hashlib.file_digest(stream, "sha512").hexdigest()
    if actual != expected:
        raise SystemExit("Debian cloud image checksum mismatch")
    with tempfile.TemporaryDirectory(prefix="limeos-p01-vm-") as tmp:
        directory = Path(tmp)
        key = directory / "key"
        run("ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-f", str(key))
        public = key.with_suffix(".pub").read_text().strip()
        (directory / "user-data").write_text(
            "#cloud-config\ndisable_root: false\nssh_pwauth: false\nusers:\n  - name: root\n    ssh_authorized_keys:\n      - "
            + public
            + "\n"
        )
        (directory / "meta-data").write_text(
            "instance-id: limeos-p01-test\nlocal-hostname: limeos-p01-test\n"
        )
        run(
            "genisoimage",
            "-quiet",
            "-output",
            str(directory / "seed.iso"),
            "-volid",
            "cidata",
            "-joliet",
            "-rock",
            str(directory / "user-data"),
            str(directory / "meta-data"),
        )
        run(
            "qemu-img",
            "create",
            "-q",
            "-f",
            "qcow2",
            "-F",
            "qcow2",
            "-b",
            str(args.image.resolve()),
            str(directory / "disk.qcow2"),
            "8G",
        )
        storage_drives = []
        if args.storage_disks:
            for index in range(4):
                disk = directory / f"storage-{index}.qcow2"
                run("qemu-img", "create", "-q", "-f", "qcow2", str(disk), "128M")
                storage_drives.extend(
                    [
                        "-drive",
                        f"file={disk},format=qcow2,if=none,id=storage{index}",
                        "-device",
                        f"virtio-blk-pci,drive=storage{index},serial=limeos-test-{index}",
                    ]
                )
        with socket.socket() as probe:
            probe.bind(("127.0.0.1", 0))
            port = probe.getsockname()[1]
        common = [
            "-i",
            str(key),
            "-o",
            "StrictHostKeyChecking=no",
            "-o",
            f"UserKnownHostsFile={directory / 'known_hosts'}",
            "-o",
            "ConnectTimeout=2",
            "-o",
            "BatchMode=yes",
            "-o",
            "LogLevel=ERROR",
        ]
        ssh = ["ssh", *common, "-p", str(port), "root@127.0.0.1"]
        with (directory / "console.log").open("wb") as console:
            vm = subprocess.Popen(
                [
                    "qemu-system-x86_64",
                    "-enable-kvm",
                    "-cpu",
                    "host",
                    "-smp",
                    "2",
                    "-m",
                    "2048" if args.build_bundle else "1024",
                    "-display",
                    "none",
                    "-serial",
                    "stdio",
                    "-drive",
                    f"file={directory / 'disk.qcow2'},format=qcow2,if=none,id=system",
                    "-device",
                    "virtio-blk-pci,drive=system,bootindex=1",
                    "-drive",
                    f"file={directory / 'seed.iso'},media=cdrom,readonly=on",
                    "-netdev",
                    f"user,id=n1,hostfwd=tcp:127.0.0.1:{port}-:22",
                    "-device",
                    "virtio-net-pci,netdev=n1",
                    *storage_drives,
                ],
                stdout=console,
                stderr=console,
            )
            try:
                deadline = time.monotonic() + 180
                while time.monotonic() < deadline:
                    result = subprocess.run(
                        [*ssh, "test -f /var/lib/cloud/instance/boot-finished"],
                        capture_output=True,
                        check=False,
                        timeout=10,
                    )
                    if result.returncode == 0:
                        break
                    if vm.poll() is not None:
                        raise RuntimeError(
                            "Guest exited: "
                            + (directory / "console.log").read_text()[-4000:]
                        )
                    time.sleep(1)
                else:
                    failure = args.output.with_name(
                        args.output.stem + "-boot-failure.txt"
                    )
                    failure.parent.mkdir(parents=True, exist_ok=True)
                    failure.write_text(
                        (directory / "console.log").read_text()[-16384:]
                        + "\nSSH: "
                        + result.stderr.decode(errors="replace")[-4096:]
                    )
                    raise RuntimeError(
                        "Guest SSH readiness timeout; console: " + str(failure)
                    )
                print(
                    "Debian guest ready; testing signed apt installation and failure recovery.",
                    flush=True,
                )
                run(
                    "scp",
                    *common,
                    "-P",
                    str(port),
                    "-r",
                    str(args.repository.resolve()),
                    "root@127.0.0.1:/opt/limeos-repo",
                )
                if args.previous_repository:
                    run(
                        "scp",
                        *common,
                        "-P",
                        str(port),
                        "-r",
                        str(args.previous_repository.resolve()),
                        "root@127.0.0.1:/opt/limeos-previous-repo",
                    )
                run(
                    "scp",
                    *common,
                    "-P",
                    str(port),
                    str(args.guest_script),
                    "root@127.0.0.1:/root/guest.py",
                )
                if args.guest_script.name in [
                    "p03_lifecycle_guest.py",
                    "p03_compose_guest.py",
                    "p03_reference_guest.py",
                    "p04_locks_guest.py",
                    "p04_locks_container_guest.py",
                ]:
                    run(
                        "scp",
                        *common,
                        "-P",
                        str(port),
                        str(args.guest_script.with_name("p03_guest.py")),
                        "root@127.0.0.1:/root/p03_guest.py",
                    )
                if args.guest_script.name in [
                    "p03_compose_guest.py",
                    "p03_reference_guest.py",
                    "p04_locks_container_guest.py",
                ]:
                    run(
                        "scp",
                        *common,
                        "-P",
                        str(port),
                        str(args.guest_script.with_name("p03_lifecycle_guest.py")),
                        "root@127.0.0.1:/root/p03_lifecycle_guest.py",
                    )
                    run(
                        "scp",
                        *common,
                        "-P",
                        str(port),
                        str(ROOT / "tests/fixtures/compose-catalog.json"),
                        "root@127.0.0.1:/root/compose-catalog-fixture.json",
                    )
                if args.guest_script.name == "p03_reference_guest.py":
                    for source, destination in [
                        (
                            args.guest_script.with_name("p03_compose_guest.py"),
                            "p03_compose_guest.py",
                        ),
                        (
                            ROOT / "tests/fixtures/wybie-layout.json",
                            "wybie-layout.json",
                        ),
                    ]:
                        run(
                            "scp",
                            *common,
                            "-P",
                            str(port),
                            str(source),
                            "root@127.0.0.1:/root/" + destination,
                        )
                if args.guest_script.name in [
                    "p04_storage_guest.py",
                    "p04_planning_guest.py",
                    "p04_targets_guest.py",
                    "p04_locks_guest.py",
                ]:
                    run(
                        "scp",
                        *common,
                        "-P",
                        str(port),
                        str(ROOT / "tests/fixtures/werkzeug-hashes.json"),
                        "root@127.0.0.1:/root/werkzeug-hashes.json",
                    )
                if args.guest_script.name in [
                    "p04_planning_guest.py",
                    "p04_targets_guest.py",
                    "p04_locks_guest.py",
                ]:
                    run(
                        "scp",
                        *common,
                        "-P",
                        str(port),
                        str(args.guest_script.with_name("p04_storage_guest.py")),
                        "root@127.0.0.1:/root/p04_storage_guest.py",
                    )
                if args.guest_script.name in [
                    "p04_targets_guest.py",
                    "p04_locks_guest.py",
                ]:
                    run(
                        "scp",
                        *common,
                        "-P",
                        str(port),
                        str(args.guest_script.with_name("p04_planning_guest.py")),
                        "root@127.0.0.1:/root/p04_planning_guest.py",
                    )
                if args.guest_script.name == "p04_locks_guest.py":
                    run(
                        "scp",
                        *common,
                        "-P",
                        str(port),
                        str(args.guest_script.with_name("p04_targets_guest.py")),
                        "root@127.0.0.1:/root/p04_targets_guest.py",
                    )
                if args.build_bundle:
                    if args.retain_previous_repository:
                        run(*ssh, "mv /opt/limeos-repo /opt/limeos-previous-repo")
                    run(
                        "scp",
                        *common,
                        "-P",
                        str(port),
                        str(args.build_bundle),
                        "root@127.0.0.1:/root/build.tar.gz",
                    )
                    run(
                        *ssh,
                        "mkdir /root/build && tar -xzf /root/build.tar.gz -C /root/build && python3 /root/build/source/tests/privileged_vm/build_guest.py --version "
                        + shlex.quote(args.package_version),
                    )
                    if args.build_output:
                        args.build_output.mkdir(parents=True, exist_ok=True)
                        run(
                            "scp",
                            *common,
                            "-P",
                            str(port),
                            "-r",
                            "root@127.0.0.1:/root/build/packages",
                            str(args.build_output),
                        )
                        run(
                            "scp",
                            *common,
                            "-P",
                            str(port),
                            "-r",
                            "root@127.0.0.1:/opt/limeos-repo",
                            str(args.build_output),
                        )
                        for name in [
                            "limeos-core",
                            "limeos-password-worker",
                            "limeos-executor",
                            "limeosctl",
                        ]:
                            run(
                                "scp",
                                *common,
                                "-P",
                                str(port),
                                f"root@127.0.0.1:/root/build/source/target/release/{name}",
                                str(args.build_output / name),
                            )
                try:
                    run(
                        *ssh,
                        "python3 /root/guest.py /opt/limeos-repo /root/result.json",
                    )
                except subprocess.CalledProcessError:
                    diagnostics = subprocess.run(
                        [
                            *ssh,
                            "systemctl show limeos-core limeos-containerd limeos-storage-ready limeos-storage-reader -p Result -p ExecMainStatus -p NRestarts -p StartLimitBurst -p StartLimitIntervalUSec; journalctl -u limeos-core -u limeos-containerd -u limeos-storage-ready -u limeos-storage-reader -u docker --no-pager -n 100",
                        ],
                        capture_output=True,
                        text=True,
                        check=False,
                    )
                    failure_name = (
                        "vm-failure.txt"
                        if args.output.name == "vm-result.json"
                        else args.output.stem.removesuffix("-vm-result")
                        + "-vm-failure.txt"
                    )
                    failure = args.output.with_name(failure_name)
                    failure.parent.mkdir(parents=True, exist_ok=True)
                    failure.write_text(
                        diagnostics.stdout[-16384:] + diagnostics.stderr[-4096:]
                    )
                    print("Guest failure diagnostics: " + str(failure), flush=True)
                    raise
                args.output.parent.mkdir(parents=True, exist_ok=True)
                run(
                    "scp",
                    *common,
                    "-P",
                    str(port),
                    "root@127.0.0.1:/root/result.json",
                    str(args.output),
                )
                evidence = json.loads(args.output.read_text())
                evidence["image"] = {"name": args.image.name, "sha512": actual}
                args.output.write_text(json.dumps(evidence, indent=2) + "\n")
                print("VM checks passed; evidence: " + str(args.output), flush=True)
            except (
                RuntimeError,
                subprocess.CalledProcessError,
                subprocess.TimeoutExpired,
            ):
                startup = args.output.with_name(
                    args.output.stem + "-startup-console.txt"
                )
                startup.parent.mkdir(parents=True, exist_ok=True)
                startup.write_text((directory / "console.log").read_text()[-32768:])
                raise
            finally:
                vm.terminate()
                try:
                    vm.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    vm.kill()
                    vm.wait()


if __name__ == "__main__":
    main()
