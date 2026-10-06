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
import hashlib
import json
import re
import secrets
import shlex
import socket
import subprocess
import sys
import time
from datetime import datetime, timedelta, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
STATE = ROOT / ".cache/arm64-qual"
REMOTE_BASE = "limeos-arm64-qual"
IMAGE = "image/debian-12-genericcloud-arm64.qcow2"
SUPERVISOR = Path(__file__).resolve().with_name("guest_supervisor.py")
# Production hosts are refused unless a recorded authorization names them.
PRODUCTION_HOSTS = ("wybie",)
AUTHORIZATION_KEYS = {
    "version",
    "host",
    "not_before",
    "not_after",
    "authorized_by",
    "reference",
    "scope",
}
MAX_WINDOW = timedelta(hours=12)
# Guests are signalled before the window closes: SIGTERM, then SIGKILL.
TERMINATE_MARGIN = timedelta(minutes=5)
KILL_MARGIN = timedelta(minutes=3)
# A new guest needs at least this much of the window left.
BOOT_MARGIN = timedelta(minutes=15)


def utc_now():
    return datetime.now(timezone.utc)


def parse_utc(text):
    if not isinstance(text, str) or not re.fullmatch(
        r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z", text
    ):
        raise ValueError(f"timestamp must be UTC like 2026-10-07T06:00:00Z: {text!r}")
    return datetime.strptime(text, "%Y-%m-%dT%H:%M:%SZ").replace(tzinfo=timezone.utc)


def production_host(host, resolve=True):
    """Whether host names a production machine, directly, by alias or by address."""
    name = host.rsplit("@", 1)[-1]
    if name.split(".", 1)[0].casefold() in PRODUCTION_HOSTS:
        return True
    if not resolve:
        return False
    # `ssh -G` prints the effective configuration without connecting.
    config = subprocess.run(
        ["ssh", "-G", host], capture_output=True, text=True, timeout=10, check=False
    ).stdout
    target = next(
        (
            line.split()[1]
            for line in config.splitlines()
            if line.startswith("hostname ")
        ),
        name,
    )
    if target.split(".", 1)[0].casefold() in PRODUCTION_HOSTS:
        return True

    def addresses(value):
        try:
            return {info[4][0] for info in socket.getaddrinfo(value, None)}
        except OSError:
            return set()

    mine = addresses(target)
    return any(mine & addresses(production) for production in PRODUCTION_HOSTS)


def load_authorization(path, host, now=None):
    """Validate a recorded authorization window for exactly this host."""
    now = now or utc_now()
    try:
        raw = Path(path).read_bytes()
        record = json.loads(raw)
    except (OSError, ValueError) as error:
        raise ValueError(f"authorization unreadable: {error}") from None
    if not isinstance(record, dict) or set(record) != AUTHORIZATION_KEYS:
        raise ValueError(
            f"authorization must have exactly {sorted(AUTHORIZATION_KEYS)}"
        )
    if record["version"] != 1:
        raise ValueError("unsupported authorization version")
    if record["host"] != host:
        raise ValueError(f"authorization names {record['host']!r}, not {host!r}")
    for key in ("authorized_by", "reference", "scope"):
        if not isinstance(record[key], str) or not record[key].strip():
            raise ValueError(f"authorization {key} is required")
    start, end = parse_utc(record["not_before"]), parse_utc(record["not_after"])
    if not start < end <= start + MAX_WINDOW:
        raise ValueError("authorization window must be positive and at most 12 hours")
    if now < start:
        raise ValueError("authorization window has not started")
    if now >= end:
        raise ValueError("authorization window has expired")
    return {**record, "sha256": hashlib.sha256(raw).hexdigest(), "end": end}


def run(*args, **kwargs):
    return subprocess.run(args, check=True, **kwargs)


class Guest:
    def __init__(self, name, host, authorization=None, resolve=True):
        if not re.fullmatch(r"[a-z0-9][a-z0-9-]{0,63}", name):
            raise ValueError(
                "guest name must use lowercase letters, digits and hyphens (max 64)"
            )
        if not re.fullmatch(r"(?:[A-Za-z0-9_.-]+@)?[A-Za-z0-9][A-Za-z0-9_.-]*", host):
            raise ValueError("host must be an explicit SSH alias or user@hostname")
        self.authorization = None
        if production_host(host, resolve=resolve):
            if authorization is None:
                raise ValueError(
                    "production host requires a recorded --authorization window"
                )
            self.authorization = load_authorization(authorization, host)
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

    def authorize(self):
        """Refuse any further SSH once a production window has closed."""
        if self.authorization and utc_now() >= self.authorization["end"]:
            raise SystemExit("authorization window expired; no further host access")

    def ssh(self, command, check=True, **kwargs):
        self.authorize()
        return subprocess.run(
            ["ssh", *self.options(), "-p", str(self.port), "root@127.0.0.1", command],
            check=check,
            **kwargs,
        )

    def push(self, source, destination):
        self.authorize()
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
        self.authorize()
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
        self.authorize()
        check = kwargs.pop("check", True)
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
            check=check,
            **kwargs,
        )


def host_info(guest):
    result = guest.host_run(
        "uname -a; getconf PAGESIZE; cat /proc/cpuinfo; cat /proc/meminfo; "
        "cat /proc/loadavg; cat /proc/pressure/memory; "
        "lsblk -d -o NAME,SIZE,ROTA,MODEL; systemctl list-units --state=running --no-pager; "
        # Read-only prerequisite and workload evidence; nothing here changes the host.
        "echo '@@prerequisites'; ls -l /dev/kvm; "
        "for t in qemu-system-aarch64 qemu-img python3; do command -v $t || echo missing:$t; done; "
        "ls /usr/share/AAVMF/AAVMF_CODE.fd /usr/share/AAVMF/AAVMF_VARS.fd 2>&1; "
        "dpkg-query -W -f '${Package} ${Version} ${Status}\\n' qemu-system-arm qemu-efi-aarch64 qemu-utils 2>&1; "
        f"ls -la ~/{REMOTE_BASE} ~/{REMOTE_BASE}/image 2>&1; df -h ~ /tmp; free -m; "
        "echo '@@qemu'; pgrep -a qemu || echo none; "
        "echo '@@docker'; docker ps --format '{{.Names}} {{.Status}}' 2>&1",
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


def deadlines(guest, terminate_at, now=None):
    """SIGTERM and SIGKILL times for a new guest, inside any authorization window."""
    now = now or utc_now()
    grace = TERMINATE_MARGIN - KILL_MARGIN
    if guest.authorization:
        end = guest.authorization["end"]
        terminate = end - TERMINATE_MARGIN
        if terminate_at is not None:
            terminate = min(terminate, terminate_at)
        elif terminate - now < BOOT_MARGIN:
            raise SystemExit("less than 15 minutes remain in the window; not booting")
        kill = min(terminate + grace, end - KILL_MARGIN)
    else:
        if terminate_at is None:
            raise SystemExit(
                "--terminate-at is required without an authorization window"
            )
        terminate, kill = terminate_at, terminate_at + grace
    if terminate <= now + timedelta(seconds=60):
        raise SystemExit("guest deadline leaves no usable time")
    return terminate, kill


def boot(args):
    guest = Guest(args.name, args.host, args.authorization)
    terminate_at, kill_at = deadlines(
        guest, parse_utc(args.terminate_at) if args.terminate_at else None
    )
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
    marker = f"limeos-arm64-{args.name}-{secrets.token_hex(8)}"
    qemu = [
        "sudo",
        "nice",
        "-n",
        "19",
        "ionice",
        "-c",
        "3",
        "qemu-system-aarch64",
        # Unique argv marker binds the host-side supervisor to this guest.
        "-name",
        marker,
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
    guest.host_run(f"mkdir -p {guest.remote}")
    run(
        "scp",
        "-o",
        "BatchMode=yes",
        str(SUPERVISOR),
        f"{args.host}:{guest.remote}/guest_supervisor.py",
    )
    guest.host_run(script)
    # Bind the supervisor to the daemonized QEMU process before anything else.
    # Newline-separated so only the supervisor itself runs in the background.
    supervise = "\n".join(
        [
            "set -e",
            f"cd {guest.remote}",
            "pid=$(sudo cat qemu.pid)",
            'start=$(sed "s/.*) //" /proc/$pid/stat | cut -d" " -f20)',
            "uid=$(awk '/^Uid:/{print $2}' /proc/$pid/status)",
            'test "$uid" = "$(id -u)"',
            (
                "setsid nohup python3 guest_supervisor.py --pid $pid --start-ticks $start "
                f"--uid $uid --marker {shlex.quote(marker)} "
                f"--terminate-at {terminate_at.timestamp():.0f} "
                f"--kill-at {kill_at.timestamp():.0f} "
                "--log supervisor.log < /dev/null > supervisor.out 2>&1 &"
            ),
            "echo $! > supervisor.pid",
            # QEMU deletes qemu.pid when it exits; keep the bound PID for stop.
            "echo $pid > qemu.bound-pid",
            "sleep 1",
            'grep -q \'"event": "bound"\' supervisor.log',
            "echo qemu=$pid supervisor=$(cat supervisor.pid)",
        ]
    )
    try:
        bound = guest.host_run(supervise, capture_output=True, text=True, timeout=60)
    except (subprocess.CalledProcessError, subprocess.TimeoutExpired):
        # Never leave a guest running without its deadline supervisor.
        guest.host_run(
            f"cd {guest.remote} && pid=$(sudo cat qemu.pid) && "
            'test "$(sudo readlink /proc/$pid/cwd)" = "$PWD" && sudo kill $pid',
            check=False,
        )
        raise SystemExit("supervisor did not bind; guest stopped")
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
                "marker": marker,
                "supervisor": {
                    "sha256": hashlib.sha256(SUPERVISOR.read_bytes()).hexdigest(),
                    "terminate_at": terminate_at.strftime("%Y-%m-%dT%H:%M:%SZ"),
                    "kill_at": kill_at.strftime("%Y-%m-%dT%H:%M:%SZ"),
                    "bound": bound.stdout.strip(),
                },
                "authorization": (
                    {
                        k: guest.authorization[k]
                        for k in sorted(AUTHORIZATION_KEYS | {"sha256"})
                    }
                    if guest.authorization
                    else None
                ),
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
    guest = Guest(args.name, args.host, args.authorization)
    if guest.port is None:
        raise SystemExit("recorded guest state required for stop")
    (guest.local / "host-after.json").write_text(
        json.dumps(host_info(guest), indent=2) + "\n"
    )
    guest.host_run(
        # The pidfile is root-owned (written before -runas drops privileges).
        # A supervisor may already have stopped the guest at its deadline.
        f"cd {guest.remote} && "
        "pid=$(cat qemu.bound-pid 2>/dev/null || sudo cat qemu.pid 2>/dev/null || true) && "
        'if [ -n "$pid" ] && [ -e /proc/$pid ]; then test "$(sudo readlink /proc/$pid/cwd)" = "$PWD" && sudo kill $pid; fi && '
        'for i in $(seq 1 60); do [ -n "$pid" ] && [ -e /proc/$pid ] || break; sleep 1; done && '
        '{ [ -z "$pid" ] || [ ! -e /proc/$pid ]; } && '
        # The supervisor exits on its own once its pidfd reports the exit.
        "spid=$(cat supervisor.pid 2>/dev/null || true) && "
        'for i in $(seq 1 30); do [ -n "$spid" ] && grep -qa guest_supervisor.py /proc/$spid/cmdline 2>/dev/null || break; sleep 1; done && '
        f"{{ sudo cat console.log > /tmp/limeos-arm64-{args.name}-console.log 2>/dev/null; "
        f"cp supervisor.log /tmp/limeos-arm64-{args.name}-supervisor.log 2>/dev/null; true; }}"
    )
    run(
        "scp",
        "-o",
        "BatchMode=yes",
        f"{args.host}:/tmp/limeos-arm64-{args.name}-console.log",
        str(guest.local / "console.log"),
    )
    run(
        "scp",
        "-o",
        "BatchMode=yes",
        f"{args.host}:/tmp/limeos-arm64-{args.name}-supervisor.log",
        str(guest.local / "supervisor.log"),
    )
    guest.host_run(
        f"rm -rf {guest.remote} /tmp/limeos-arm64-{args.name}-console.log "
        f"/tmp/limeos-arm64-{args.name}-supervisor.log"
    )
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
    parser.add_argument(
        "--authorization",
        type=Path,
        help="Recorded JSON window required for a production host",
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
    b.add_argument(
        "--terminate-at",
        help="Earlier UTC guest deadline, e.g. for a short deadline proof",
    )
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
    hexec = sub.add_parser(
        "host-exec",
        help="Run one recorded host command (operator-approved preparation/cleanup)",
    )
    hexec.add_argument("--log", type=Path, required=True, help="JSON lines record")
    hexec.add_argument("host_command")
    info = sub.add_parser(
        "host-info", help="Capture read-only host pressure/platform evidence"
    )
    info.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.command == "host-exec":
        guest = Guest("host-exec", args.host, args.authorization)
        started = utc_now()
        result = guest.host_run(
            args.host_command, check=False, capture_output=True, text=True, timeout=1800
        )
        with args.log.open("a") as stream:
            stream.write(
                json.dumps(
                    {
                        "started": started.strftime("%Y-%m-%dT%H:%M:%SZ"),
                        "host": args.host,
                        "authorization_sha256": (guest.authorization or {}).get(
                            "sha256"
                        ),
                        "command": args.host_command,
                        "exit": result.returncode,
                        "stdout": result.stdout[-65536:],
                        "stderr": result.stderr[-16384:],
                    }
                )
                + "\n"
            )
        sys.stdout.write(result.stdout)
        sys.stderr.write(result.stderr)
        sys.exit(result.returncode)
    if args.command == "host-info":
        args.output.write_text(
            json.dumps(
                host_info(Guest("host-info", args.host, args.authorization)), indent=2
            )
            + "\n"
        )
    elif args.command == "boot":
        boot(args)
    elif args.command == "stop":
        stop(args)
    else:
        guest = Guest(args.name, args.host, args.authorization)
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
