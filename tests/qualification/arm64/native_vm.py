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

    def dispatch_options(self, kwargs=None):
        options = dict(kwargs or {})
        self.authorize()
        if self.authorization:
            remaining = (self.authorization["end"] - utc_now()).total_seconds() - 1
            if remaining <= 0:
                raise SystemExit("authorization window expired or too short for access")
            limit = options.get("timeout")
            options["timeout"] = (
                min(limit, remaining) if limit is not None else remaining
            )
        return options

    def ssh(self, command, check=True, **kwargs):
        kwargs = self.dispatch_options(kwargs)
        return subprocess.run(
            ["ssh", *self.options(), "-p", str(self.port), "root@127.0.0.1", command],
            check=check,
            **kwargs,
        )

    def push(self, source, destination):
        options = self.dispatch_options()
        run(
            "scp",
            *self.options(),
            "-P",
            str(self.port),
            "-r",
            str(source),
            f"root@127.0.0.1:{destination}",
            **options,
        )

    def pull(self, source, destination):
        Path(destination).parent.mkdir(parents=True, exist_ok=True)
        options = self.dispatch_options()
        run(
            "scp",
            *self.options(),
            "-P",
            str(self.port),
            "-r",
            f"root@127.0.0.1:{source}",
            str(destination),
            **options,
        )

    def host_run(self, command, **kwargs):
        check = kwargs.pop("check", True)
        kwargs = self.dispatch_options(kwargs)
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

    def host_push(self, source, destination):
        options = self.dispatch_options()
        return run(
            "scp",
            "-o",
            "BatchMode=yes",
            str(source),
            f"{self.host}:{destination}",
            **options,
        )

    def host_pull(self, source, destination):
        Path(destination).parent.mkdir(parents=True, exist_ok=True)
        options = self.dispatch_options()
        return run(
            "scp",
            "-o",
            "BatchMode=yes",
            f"{self.host}:{source}",
            str(destination),
            **options,
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
    guest.host_push(guest.local / "seed.iso", f"{guest.remote}/seed.iso")
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
    host_identity = guest.host_run(
        f"cd {guest.remote} && pwd && id -u && id -un",
        capture_output=True,
        text=True,
        timeout=30,
    ).stdout.splitlines()
    if (
        len(host_identity) != 3
        or not host_identity[0].startswith("/")
        or not host_identity[1].isdigit()
        or not re.fullmatch(r"[a-z_][a-z0-9_-]{0,31}", host_identity[2])
    ):
        raise SystemExit("host directory and account identity unavailable")
    work_directory, host_uid, host_user = host_identity
    qemu = [
        "/usr/bin/nice",
        "-n",
        "19",
        "/usr/bin/ionice",
        "-c",
        "3",
        "/usr/bin/qemu-system-aarch64",
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
        host_user,
    ]
    script = " && ".join(
        [
            f"cd {guest.remote}",
            f"qemu-img create -q -f qcow2 -F qcow2 -b $HOME/{REMOTE_BASE}/{IMAGE} disk.qcow2 {args.disk}",
            "cp /usr/share/AAVMF/AAVMF_VARS.fd vars.fd",
            "touch console.log",
            *[d.replace(f"{guest.remote}/", "") for d in disks],
        ]
    )
    guest.host_run(f"mkdir -p {guest.remote}")
    guest.host_push(SUPERVISOR, f"{guest.remote}/guest_supervisor.py")
    guest.host_run(script)
    command_file = guest.local / "launch.json"
    command_file.write_text(json.dumps({"argv": qemu, "cwd": work_directory}) + "\n")
    guest.host_push(command_file, f"{guest.remote}/launch.json")
    control = guest.host_run(
        "sudo -n /usr/bin/mktemp -d -- /tmp/limeos-arm64-supervisor-XXXXXXXX",
        capture_output=True,
        text=True,
        timeout=30,
    ).stdout.strip()
    if not re.fullmatch(r"/tmp/limeos-arm64-supervisor-[A-Za-z0-9]{8}", control):
        raise SystemExit("root-controlled supervisor directory unavailable")
    # Root executes only protected copies whose bytes match the local inputs.
    copied = guest.host_run(
        f"sudo -n /usr/bin/install -o root -g root -m 0500 {guest.remote}/guest_supervisor.py {control}/guest_supervisor.py && "
        f"sudo -n /usr/bin/install -o root -g root -m 0400 {guest.remote}/launch.json {control}/launch.json && "
        f"sudo -n /usr/bin/sha256sum {control}/guest_supervisor.py {control}/launch.json",
        capture_output=True,
        text=True,
        timeout=30,
    ).stdout.splitlines()
    supervisor_sha = hashlib.sha256(SUPERVISOR.read_bytes()).hexdigest()
    if [line.split()[0] for line in copied] != [
        supervisor_sha,
        hashlib.sha256(command_file.read_bytes()).hexdigest(),
    ]:
        raise SystemExit("protected launcher inputs differ; no guest launched")
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
                    "sha256": supervisor_sha,
                    "terminate_at": terminate_at.strftime("%Y-%m-%dT%H:%M:%SZ"),
                    "kill_at": kill_at.strftime("%Y-%m-%dT%H:%M:%SZ"),
                    "control_directory": control,
                    "bound": None,
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
    not_after = (
        guest.authorization["end"]
        if guest.authorization
        else kill_at + timedelta(seconds=1)
    )
    launch = "\n".join(
        [
            "set -euo pipefail",
            f"cd {control}",
            (
                "/usr/bin/setsid /usr/bin/nohup /usr/bin/env -i PATH=/usr/sbin:/usr/bin:/sbin:/bin LC_ALL=C PYTHONDONTWRITEBYTECODE=1 "
                f"/usr/bin/python3 -I {control}/guest_supervisor.py launch "
                f"--command {control}/launch.json --state {control}/process.json --log {control}/supervisor.log "
                f"--uid {host_uid} --marker {shlex.quote(marker)} "
                f"--terminate-at {terminate_at.timestamp():.0f} --kill-at {kill_at.timestamp():.0f} "
                f"--not-after {not_after.timestamp():.0f} < /dev/null > supervisor.out 2>&1 &"
            ),
            "echo $! > supervisor.pid",
        ]
    )
    # This one detached host operation starts the owner first. It alone spawns
    # foreground QEMU and owns cleanup even if this SSH/workstation disappears.
    guest.host_run(
        "sudo -n /usr/bin/env -i PATH=/usr/sbin:/usr/bin:/sbin:/bin LC_ALL=C "
        f"/bin/bash --noprofile --norc -c {shlex.quote(launch)}",
        timeout=30,
    )
    bound_until = time.monotonic() + 15
    while time.monotonic() < bound_until:
        result = guest.host_run(
            f"sudo -n /usr/bin/cat {control}/process.json",
            check=False,
            capture_output=True,
            text=True,
            timeout=30,
        )
        if result.returncode == 0:
            bound = json.loads(result.stdout)
            if bound.get("marker") != marker or bound.get("uid") != int(host_uid):
                raise SystemExit("launcher identity differs; inspect protected state")
            metadata = json.loads((guest.local / "guest.json").read_text())
            metadata["supervisor"]["bound"] = bound
            (guest.local / "guest.json").write_text(
                json.dumps(metadata, indent=2) + "\n"
            )
            break
        time.sleep(0.2)
    else:
        raise SystemExit("launcher has no published guest; inspect its protected log")
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
    metadata = json.loads((guest.local / "guest.json").read_text())
    marker = metadata.get("marker")
    if not marker:
        raise SystemExit("guest has no recorded QEMU marker; stop it on the host")
    control = metadata.get("supervisor", {}).get("control_directory", "")
    if not re.fullmatch(r"/tmp/limeos-arm64-supervisor-[A-Za-z0-9]{8}", control):
        raise SystemExit("protected launcher state required; no numeric PID fallback")
    (guest.local / "host-after.json").write_text(
        json.dumps(host_info(guest), indent=2) + "\n"
    )
    guest.host_run(
        f"cd {guest.remote} && "
        # pidfd signals cannot reach a reused PID; the helper also checks kernel
        # start time, UID and exact marker argv before sending any signal.
        f"sudo -n /usr/bin/env -i PATH=/usr/sbin:/usr/bin:/sbin:/bin /usr/bin/python3 -I {control}/guest_supervisor.py stop {control}/process.json --marker {shlex.quote(marker)} && "
        f"sudo -n /usr/bin/env -i PATH=/usr/sbin:/usr/bin:/sbin:/bin /usr/bin/python3 -I {control}/guest_supervisor.py wait {control}/owner.json --marker {shlex.quote(marker)} && "
        f"{{ sudo -n /usr/bin/cat console.log > /tmp/limeos-arm64-{args.name}-console.log 2>/dev/null; "
        f"sudo -n /usr/bin/cat {control}/supervisor.log > /tmp/limeos-arm64-{args.name}-supervisor.log; }}"
    )
    guest.host_pull(
        f"/tmp/limeos-arm64-{args.name}-console.log", guest.local / "console.log"
    )
    guest.host_pull(
        f"/tmp/limeos-arm64-{args.name}-supervisor.log", guest.local / "supervisor.log"
    )
    guest.host_run(
        f"sudo -n /usr/bin/rm -rf -- {control} && rm -rf {guest.remote} /tmp/limeos-arm64-{args.name}-console.log "
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
