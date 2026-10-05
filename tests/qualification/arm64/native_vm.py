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
        self.name = name
        self.host = host
        self.local = STATE / name
        self.remote = f"{REMOTE_BASE}/runs/{name}"
        self.key = self.local / "key"
        meta = self.local / "guest.json"
        self.port = json.loads(meta.read_text())["port"] if meta.exists() else None

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
            ["ssh", "-o", "BatchMode=yes", self.host, command], check=True, **kwargs
        )


def boot(args):
    guest = Guest(args.name, args.host)
    if guest.local.exists():
        raise SystemExit(f"{guest.local} exists; stop that guest first")
    guest.local.mkdir(parents=True)
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
    guest.host_run(
        # The pidfile is root-owned (written before -runas drops privileges).
        f"cd {guest.remote} && pid=$(sudo cat qemu.pid) && sudo kill $pid && "
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
    parser.add_argument("--host", default="holly@wybie")
    sub = parser.add_subparsers(dest="command", required=True)
    b = sub.add_parser("boot")
    b.add_argument("name")
    b.add_argument("--port", type=int, required=True)
    b.add_argument("--cpus", type=int, default=3)
    b.add_argument("--memory", type=int, default=1536)
    b.add_argument("--disk", default="24G")
    b.add_argument("--storage-disks", action="store_true")
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
    args = parser.parse_args()
    if args.command == "boot":
        boot(args)
    elif args.command == "stop":
        stop(args)
    else:
        guest = Guest(args.name, args.host)
        if args.command == "exec":
            sys.exit(guest.ssh(args.remote_command, check=False).returncode)
        if args.command == "push":
            guest.push(args.source, args.destination)
        if args.command == "pull":
            guest.pull(args.source, args.destination)


if __name__ == "__main__":
    main()
