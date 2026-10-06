#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Run a disposable native ARM64 Debian 12 KVM guest on a remote Pi host.

The orchestrator runs on the workstation. The Pi only hosts QEMU: every file
and command reaches the guest through an SSH jump to a loopback-forwarded port.
Nothing from LimeOS is installed on the Pi itself. The guest's system disk is a
throwaway overlay of a checksum-verified Debian cloud image; optional storage
disks are empty 128 MiB qcow2 files with fixed serials, as in the AMD64 runner.

Host requirements: qemu-system-aarch64, qemu-img, AAVMF firmware, /dev/kvm and
passwordless sudo. QEMU opens /dev/kvm as root, then drops to the SSH user with
-runas, so no group membership or ACL on the host changes.
"""

import argparse
import json
import re
import shlex
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
STATE = ROOT / ".cache/arm64-qual"
REMOTE_BASE = "limeos-arm64-qual"
IMAGE = "image/debian-12-genericcloud-arm64.qcow2"


def run(*args, **kwargs):
    return subprocess.run(args, check=True, **kwargs)


class Guest:
    def __init__(self, name, host):
        if not re.fullmatch(r"[a-z0-9][a-z0-9-]{0,63}", name):
            raise ValueError(
                "guest name must use lowercase letters, digits and hyphens (max 64)"
            )
        if not re.fullmatch(r"(?:[A-Za-z0-9_.-]+@)?[A-Za-z0-9][A-Za-z0-9_.-]*", host):
            raise ValueError("host must be an explicit SSH alias or user@hostname")
        if host.rsplit("@", 1)[-1].split(".", 1)[0].casefold() == "wybie":
            raise ValueError(
                "wybie is in production and is excluded from qualification"
            )
        self.name = name
        self.host = host
        self.local = STATE / name
        self.remote = f"{REMOTE_BASE}/runs/{name}"
        self.key = self.local / "key"
        meta = self.local / "guest.json"
        stored = json.loads(meta.read_text()) if meta.exists() else None
        if stored and stored["host"] != host:
            raise ValueError("host differs from recorded guest host")
        self.port = stored["port"] if stored else None

    def options(self):
        return [
            "-i",
            str(self.key),
            "-o",
            f"ProxyJump={self.host}",
            "-o",
            "StrictHostKeyChecking=no",
            "-o",
            f"UserKnownHostsFile={self.local / 'known_hosts'}",
            "-o",
            "BatchMode=yes",
            "-o",
            "ConnectTimeout=10",
            "-o",
            "ServerAliveInterval=30",
            "-o",
            "LogLevel=ERROR",
        ]

    def ssh(self, command, check=True, **kwargs):
        return subprocess.run(
            ["ssh", *self.options(), "-p", str(self.port), "root@127.0.0.1", command],
            check=check,
            **kwargs,
        )

    def push(self, source, destination):
        run(
            "scp",
            *self.options(),
            "-P",
            str(self.port),
            "-r",
            str(source),
            f"root@127.0.0.1:{destination}",
        )

    def pull(self, source, destination):
        Path(destination).parent.mkdir(parents=True, exist_ok=True)
        run(
            "scp",
            *self.options(),
            "-P",
            str(self.port),
            "-r",
            f"root@127.0.0.1:{source}",
            str(destination),
        )

    def host_run(self, command, **kwargs):
        return subprocess.run(
            [
                "ssh",
                "-o",
                "BatchMode=yes",
                "-o",
                "ConnectTimeout=10",
                self.host,
                command,
            ],
            check=True,
            **kwargs,
        )


def host_info(guest):
    result = guest.host_run(
        "uname -a; getconf PAGESIZE; cat /proc/cpuinfo; cat /proc/meminfo; "
        "cat /proc/loadavg; cat /proc/pressure/memory; "
        "lsblk -d -o NAME,SIZE,ROTA,MODEL; systemctl list-units --state=running --no-pager",
        text=True,
        capture_output=True,
        timeout=30,
    )
    return {
        "host": guest.host,
        "captured": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
        "raw": result.stdout,
        "stderr": result.stderr,
    }


def boot(args):
    guest = Guest(args.name, args.host)
    if (
        not 1 <= args.port <= 65535
        or not 1 <= args.cpus <= 3
        or not 512 <= args.memory <= 2048
    ):
        raise SystemExit("port 1..65535, CPUs 1..3 and memory 512..2048 MiB required")
    if not re.fullmatch(r"[1-9][0-9]*G", args.disk):
        raise SystemExit("disk must be an integer GiB size, e.g. 24G")
    if not re.fullmatch(r"[0-9a-f]{128}", args.image_sha512):
        raise SystemExit("verified Debian cloud image SHA-512 required")
    if guest.local.exists():
        raise SystemExit(f"{guest.local} exists; stop that guest first")
    # Read-only host checks precede all guest files and QEMU startup.
    preflight = guest.host_run(
        f'test "$(uname -m)" = aarch64 && test -c /dev/kvm && '
        f"test ! -e {guest.remote} && sha512sum {REMOTE_BASE}/{IMAGE}",
        capture_output=True,
        text=True,
        timeout=60,
    )
    if preflight.stdout.split()[0] != args.image_sha512:
        raise SystemExit("host image SHA-512 differs from verified Debian image")
    before = host_info(guest)
    guest.local.mkdir(parents=True)
    (guest.local / "host-before.json").write_text(json.dumps(before, indent=2) + "\n")
    run("ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-f", str(guest.key))
    public = guest.key.with_suffix(".pub").read_text().strip()
    # The AMD64 guest suites refuse to run unless the hostname marks a disposable VM.
    (guest.local / "user-data").write_text(
        "#cloud-config\ndisable_root: false\nssh_pwauth: false\nusers:\n"
        "  - name: root\n    ssh_authorized_keys:\n      - " + public + "\n"
    )
    (guest.local / "meta-data").write_text(
        f"instance-id: limeos-arm64-{args.name}\nlocal-hostname: limeos-p01-test\n"
    )
    run(
        "genisoimage",
        "-quiet",
        "-output",
        str(guest.local / "seed.iso"),
        "-volid",
        "cidata",
        "-joliet",
        "-rock",
        str(guest.local / "user-data"),
        str(guest.local / "meta-data"),
    )
    guest.host_run(f"mkdir -p {guest.remote}")
    run(
        "scp",
        "-o",
        "BatchMode=yes",
        str(guest.local / "seed.iso"),
        f"{args.host}:{guest.remote}/seed.iso",
    )
    storage = []
    disks = []
    if args.storage_disks:
        for index in range(4):
            disks.append(
                f"qemu-img create -q -f qcow2 {guest.remote}/storage-{index}.qcow2 128M"
            )
            storage += [
                "-drive",
                f"file=storage-{index}.qcow2,format=qcow2,if=none,id=storage{index}",
                "-device",
                f"virtio-blk-pci,drive=storage{index},serial=limeos-test-{index}",
            ]
    qemu = [
        "sudo",
        "nice",
        "-n",
        "19",
        "ionice",
        "-c",
        "3",
        "qemu-system-aarch64",
        "-machine",
        "virt,gic-version=host",
        "-accel",
        "kvm",
        "-cpu",
        "host",
        "-smp",
        str(args.cpus),
        "-m",
        str(args.memory),
        "-display",
        "none",
        "-serial",
        "file:console.log",
        "-drive",
        "if=pflash,format=raw,readonly=on,file=/usr/share/AAVMF/AAVMF_CODE.fd",
        "-drive",
        "if=pflash,format=raw,file=vars.fd",
        "-drive",
        "file=disk.qcow2,format=qcow2,if=none,id=system",
        "-device",
        "virtio-blk-pci,drive=system,bootindex=1",
        *storage,
        # The seed is a SCSI CD so storage suites still see /dev/vdb-/dev/vde.
        "-device",
        "virtio-scsi-pci,id=scsi0",
        "-drive",
        "file=seed.iso,format=raw,if=none,id=seed,media=cdrom,readonly=on",
        "-device",
        "scsi-cd,drive=seed,bus=scsi0.0",
        "-netdev",
        f"user,id=n1,hostfwd=tcp:127.0.0.1:{args.port}-:22",
        "-device",
        # No iPXE option ROM is installed on the host; the guest boots from disk.
        "virtio-net-pci,netdev=n1,romfile=",
        "-runas",
        "$(id -un)",
        "-pidfile",
        "qemu.pid",
        "-daemonize",
    ]
    script = " && ".join(
        [
            f"cd {guest.remote}",
            f"qemu-img create -q -f qcow2 -F qcow2 -b $HOME/{REMOTE_BASE}/{IMAGE} disk.qcow2 {args.disk}",
            "cp /usr/share/AAVMF/AAVMF_VARS.fd vars.fd",
            "touch console.log",
            *[d.replace(f"{guest.remote}/", "") for d in disks],
            " ".join(q if q.startswith("$(") else shlex.quote(q) for q in qemu),
        ]
    )
    guest.host_run(script)
    (guest.local / "guest.json").write_text(
        json.dumps(
            {
                "name": args.name,
                "host": args.host,
                "port": args.port,
                "cpus": args.cpus,
                "memory_mib": args.memory,
                "disk": args.disk,
                "storage_disks": args.storage_disks,
                "image_sha512": args.image_sha512,
                "qemu": " ".join(qemu),
                "started": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
            },
            indent=2,
        )
        + "\n"
    )
    guest.port = args.port
    deadline = time.monotonic() + args.timeout
    while time.monotonic() < deadline:
        result = guest.ssh(
            "test -f /var/lib/cloud/instance/boot-finished",
            check=False,
            capture_output=True,
            timeout=30,
        )
        if result.returncode == 0:
            print(f"guest {args.name} ready on {args.host}:127.0.0.1:{args.port}")
            return
        time.sleep(5)
    raise SystemExit(f"guest {args.name} did not become ready; see console.log")


def stop(args):
    guest = Guest(args.name, args.host)
    if guest.port is None:
        raise SystemExit("recorded guest state required for stop")
    (guest.local / "host-after.json").write_text(
        json.dumps(host_info(guest), indent=2) + "\n"
    )
    guest.host_run(
        # The pidfile is root-owned (written before -runas drops privileges).
        f"cd {guest.remote} && pid=$(sudo cat qemu.pid) && "
        'test "$(sudo readlink /proc/$pid/cwd)" = "$PWD" && sudo kill $pid && '
        "for i in $(seq 1 60); do [ -e /proc/$pid ] || break; sleep 1; done && "
        "[ ! -e /proc/$pid ] && "
        f"{{ sudo cat console.log > /tmp/limeos-arm64-{args.name}-console.log 2>/dev/null; true; }}"
    )
    run(
        "scp",
        "-o",
        "BatchMode=yes",
        f"{args.host}:/tmp/limeos-arm64-{args.name}-console.log",
        str(guest.local / "console.log"),
    )
    guest.host_run(f"rm -rf {guest.remote} /tmp/limeos-arm64-{args.name}-console.log")
    archive = STATE / "stopped" / args.name
    archive.parent.mkdir(parents=True, exist_ok=True)
    guest.local.rename(archive)
    print(f"guest {args.name} stopped; local state kept in {archive}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--host",
        required=True,
        help="Explicitly available isolated native ARM64 SSH host",
    )
    sub = parser.add_subparsers(dest="command", required=True)
    b = sub.add_parser("boot")
    b.add_argument("name")
    b.add_argument("--port", type=int, required=True)
    b.add_argument("--cpus", type=int, default=3)
    b.add_argument("--memory", type=int, default=1536)
    b.add_argument("--disk", default="24G")
    b.add_argument("--storage-disks", action="store_true")
    b.add_argument(
        "--image-sha512",
        required=True,
        help="Debian SHA512SUMS digest for the installed base image",
    )
    b.add_argument("--timeout", type=int, default=900)
    e = sub.add_parser("exec")
    e.add_argument("name")
    e.add_argument("remote_command")
    p = sub.add_parser("push")
    p.add_argument("name")
    p.add_argument("source")
    p.add_argument("destination")
    q = sub.add_parser("pull")
    q.add_argument("name")
    q.add_argument("source")
    q.add_argument("destination")
    s = sub.add_parser("stop")
    s.add_argument("name")
    info = sub.add_parser(
        "host-info", help="Capture read-only host pressure/platform evidence"
    )
    info.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.command == "host-info":
        args.output.write_text(
            json.dumps(host_info(Guest("host-info", args.host)), indent=2) + "\n"
        )
    elif args.command == "boot":
        boot(args)
    elif args.command == "stop":
        stop(args)
    else:
        guest = Guest(args.name, args.host)
        if guest.port is None:
            parser.error("recorded guest state required; boot the named guest first")
        if args.command == "exec":
            sys.exit(guest.ssh(args.remote_command, check=False).returncode)
        if args.command == "push":
            guest.push(args.source, args.destination)
        if args.command == "pull":
            guest.pull(args.source, args.destination)


if __name__ == "__main__":
    main()
