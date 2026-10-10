#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Run one owned, disposable AMD64 KVM guest for RW-040 combined qualification.

uv run run_guest.py --packages-dir DIR --output FRESH_DIR [--budget-total 65]
uv run run_guest.py --sweep        # remove stale owned runs after a crash

Transport is the guest serial console (captured to a file), QMP for power
control, a read-only inputs disk and a raw results disk. There is no SSH,
host port forward, host directory share or workstation Docker use. QEMU is
a direct child with a unique name marker and parent-death SIGKILL; a hard
deadline quits it through QMP and then kills it. Every owned run directory is
removed afterwards, including after failure.
"""

import argparse
import ctypes
import datetime
import errno
import hashlib
import io
import json
import os
import re
import secrets
import select
import shutil
import signal
import socket
import stat
import subprocess
import sys
import tarfile
import tempfile
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
FIXTURES = ROOT / "tests/fixtures/rw040-combined"
WORK_ROOT = ROOT / ".cache/rw040/runs"
GUEST_FILES = [
    "combined_guest.py",
    "env_probe.py",
    "selfcheck_probe.py",
    "probe_adapter.py",
]
FIXTURE_FILES = ["layout.json", "cases.json"]
RESULTS_BYTES = 64 << 20
MAX_RESULT_MEMBER = 16 << 20
MAX_RESULT_TOTAL = 48 << 20
MAX_RESULT_MEMBERS = 1024
EXPECTED_STAGES = {
    "platform",
    "install",
    "service",
    "confinement",
    "image",
    "empty",
    "layout",
    "containers",
    "ground-truth",
    "environment",
    "selfcheck",
    "transitions",
    "budget",
    "cases",
}
PR_SET_PDEATHSIG = 1
FORBIDDEN_OPTIONS = {
    "-virtfs",
    "-fsdev",
    "-chardev",
    "-monitor",
    "-incoming",
    "-loadvm",
}
FORBIDDEN_FRAGMENTS = (
    "hostfwd=",
    "guestfwd=",
    "smb=",
    "virtio-9p",
    "vhost-user-fs",
    "tcp:",
    "telnet:",
)


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def sha512(path: Path) -> str:
    digest = hashlib.sha512()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def unique_object(pairs):
    value = {}
    for key, item in pairs:
        if key in value:
            raise ValueError("duplicate JSON field")
        value[key] = item
    return value


def verify_image(image: Path, checksums: Path) -> str:
    expected = [
        line.split()[0]
        for line in checksums.read_text().splitlines()
        if line.split() and line.split()[-1].lstrip("*") == image.name
    ]
    if len(expected) != 1:
        raise SystemExit(f"{image.name}: not listed exactly once in {checksums}")
    actual = sha512(image)
    if actual != expected[0]:
        raise SystemExit(f"{image.name}: SHA-512 mismatch")
    return actual


def verify_packages(directory: Path, manifest: dict) -> list[dict]:
    verified = []
    for entry in manifest["packages"]:
        path = directory / entry["file"]
        if not path.is_file():
            raise SystemExit(f"missing package {entry['file']}")
        actual = sha256(path)
        if actual != entry["sha256"]:
            raise SystemExit(
                f"{entry['file']}: SHA-256 {actual} differs from the recorded {entry['sha256']}"
            )
        verified.append(
            {"file": entry["file"], "sha256": actual, "bytes": path.stat().st_size}
        )
    return verified


def verify_probe(directory: Path, library_source: str) -> dict:
    """Accept a closed trusted supply record, never arbitrary argv/auth inputs."""
    manifest = directory / "manifest.json"
    if manifest.stat().st_size > 4096:
        raise ValueError("probe manifest too large")
    if manifest.is_symlink() or not manifest.is_file():
        raise ValueError("probe supply manifest must be a regular file")
    record = json.loads(manifest.read_text(), object_pairs_hook=unique_object)
    fields = {
        "contract",
        "source_commit",
        "source_sha256",
        "binary",
        "binary_sha256",
        "toolchain",
        "profile",
        "library_source",
    }
    if not isinstance(record, dict) or set(record) != fields:
        raise ValueError("invalid probe supply fields")
    if (
        type(record["contract"]) is not int
        or record["contract"] != 1
        or record["binary"] != "combined_read_probe"
        or record["profile"] != "release"
        or record["toolchain"] != "1.88.0"
        or record["library_source"] != library_source
        or not re.fullmatch(r"[0-9a-f]{40}", record["source_commit"])
        or not re.fullmatch(r"[0-9a-f]{64}", record["binary_sha256"])
        or record["source_sha256"]
        != "3929400eb457937923311206201e4ef8ce2b935afc685ec7e81b27e557699b9c"
    ):
        raise ValueError("probe provenance or fixed build contract differs")
    for name, expected in (
        ("combined_read_probe.rs", record["source_sha256"]),
        ("combined_read_probe", record["binary_sha256"]),
    ):
        path = directory / name
        if path.is_symlink() or not path.is_file() or sha256(path) != expected:
            raise ValueError(f"probe input identity differs: {name}")
    return record


def deb_members(data: bytes) -> dict[str, bytes]:
    if not data.startswith(b"!<arch>\n"):
        raise ValueError("not a Debian ar archive")
    members, position = {}, 8
    while position + 60 <= len(data):
        header = data[position : position + 60]
        name, size = header[:16].decode().strip().rstrip("/"), int(header[48:58])
        members[name] = data[position + 60 : position + 60 + size]
        position += 60 + size + (size & 1)
    return members


def deb_data_tar(path: Path) -> tarfile.TarFile:
    """Open data.tar.*; Python 3.14 reads zstd, otherwise use dpkg-deb when present."""
    members = deb_members(path.read_bytes())
    name = next(n for n in members if n.startswith("data.tar"))
    try:
        return tarfile.open(fileobj=io.BytesIO(members[name]))
    except tarfile.TarError:
        if not shutil.which("dpkg-deb"):
            raise
        raw = subprocess.run(
            ["dpkg-deb", "--fsys-tarfile", str(path)], capture_output=True, check=True
        ).stdout
        return tarfile.open(fileobj=io.BytesIO(raw))


def derive_package_manifest(directory: Path, tested_source: str) -> dict:
    """For packages built in the same workflow: record, rather than trust, their identities."""
    packages = []
    for path in sorted(directory.glob("*.deb")):
        binaries = {}
        with deb_data_tar(path) as tar:
            for member in tar:
                parts = member.name.removeprefix("./").split("/")
                if (
                    member.isfile()
                    and parts[:3]
                    in (["usr", "lib", "limeos"], ["usr", "lib", "limeos-shadow"])
                    and len(parts) == 4
                ):
                    binaries[parts[3]] = hashlib.sha256(
                        tar.extractfile(member).read()
                    ).hexdigest()
        packages.append(
            {
                "file": path.name,
                "sha256": sha256(path),
                "bytes": path.stat().st_size,
                "binaries": binaries,
            }
        )
    if not packages:
        raise SystemExit(f"no .deb packages in {directory}")
    return {
        "description": "derived from packages built in the same workflow",
        "tested_source": tested_source,
        "packages": packages,
    }


def user_data() -> str:
    """No accounts, keys or SSH: one root program from the inputs disk, then power off."""
    return (
        "#cloud-config\n"
        "ssh_pwauth: false\n"
        "disable_root: true\n"
        "ssh_deletekeys: true\n"
        "bootcmd:\n"
        "  - [systemctl, mask, --now, ssh.service, ssh.socket]\n"
        "runcmd:\n"
        "  - [sh, -c, 'mkdir -p /qual/inputs && tar -xf /dev/disk/by-id/virtio-rw040-inputs -C /qual/inputs"
        " && python3 /qual/inputs/guest/combined_guest.py --inputs /qual/inputs"
        " --results /dev/disk/by-id/virtio-rw040-results']\n"
        "  - [systemctl, poweroff]\n"
    )


def meta_data(run_id: str) -> str:
    return f"instance-id: rw040-{run_id}\nlocal-hostname: rw040-guest\n"


def inputs_tar(
    path: Path, manifest: dict, packages_dir: Path, probe_dir: Path | None = None
) -> None:
    def add(tar: tarfile.TarFile, source: Path, name: str, mode: int) -> None:
        info = tar.gettarinfo(str(source), arcname=name)
        info.uid = info.gid = 0
        info.uname = info.gname = "root"
        info.mtime = 0
        info.mode = mode
        with source.open("rb") as stream:
            tar.addfile(info, stream)

    data = json.dumps(manifest, indent=2, sort_keys=True).encode()
    with tarfile.open(path, "w", format=tarfile.GNU_FORMAT) as tar:
        info = tarfile.TarInfo("manifest.json")
        info.size, info.mode, info.mtime = len(data), 0o644, 0
        tar.addfile(info, io.BytesIO(data))
        for name in GUEST_FILES:
            add(tar, HERE / "guest" / name, f"guest/{name}", 0o755)
        for name in FIXTURE_FILES:
            add(tar, FIXTURES / name, f"fixtures/{name}", 0o644)
        for entry in manifest["packages"]:
            add(tar, packages_dir / entry["file"], f"packages/{entry['file']}", 0o644)
        if probe_dir is not None:
            for name in (
                "manifest.json",
                "combined_read_probe.rs",
                "combined_read_probe",
            ):
                add(
                    tar,
                    probe_dir / name,
                    f"probe/{name}",
                    0o755 if name == "combined_read_probe" else 0o644,
                )


def qemu_argv(
    run_dir: Path, marker: str, layout: dict, memory_mib: int, cpus: int
) -> list[str]:
    argv = [
        "qemu-system-x86_64",
        "-name",
        f"{marker},process={marker}",
        "-nodefaults",
        "-no-reboot",
        "-machine",
        "q35,accel=kvm",
        "-cpu",
        "host",
        "-smp",
        str(cpus),
        "-m",
        str(memory_mib),
        "-display",
        "none",
        "-vga",
        "none",
        "-serial",
        f"file:{run_dir / 'console.log'}",
        "-qmp",
        f"unix:{run_dir / 'qmp.sock'},server=on,wait=off",
        "-device",
        "virtio-rng-pci",
        "-drive",
        f"file={run_dir / 'system.qcow2'},format=qcow2,if=none,id=system",
        "-device",
        "virtio-blk-pci,drive=system,bootindex=1",
        "-drive",
        f"file={run_dir / 'seed.iso'},format=raw,if=none,id=seed,readonly=on",
        "-device",
        "virtio-blk-pci,drive=seed",
        "-drive",
        f"file={run_dir / 'inputs.tar'},format=raw,if=none,id=inputs,readonly=on",
        "-device",
        "virtio-blk-pci,drive=inputs,serial=rw040-inputs",
        "-drive",
        f"file={run_dir / 'results.img'},format=raw,if=none,id=results",
        "-device",
        "virtio-blk-pci,drive=results,serial=rw040-results",
        "-netdev",
        "user,id=net0,restrict=off",
        "-device",
        "virtio-net-pci,netdev=net0",
    ]
    for index, disk in enumerate(layout["disks"]):
        argv += [
            "-drive",
            f"file={run_dir / f'data-{index}.qcow2'},format=qcow2,if=none,id=data{index}",
            "-device",
            f"virtio-blk-pci,drive=data{index},serial={disk['serial']}",
        ]
    check_argv(argv)
    return argv


def check_argv(argv: list[str]) -> None:
    if FORBIDDEN_OPTIONS.intersection(argv) or any(
        f in a for a in argv for f in FORBIDDEN_FRAGMENTS
    ):
        raise SystemExit(
            "refusing a guest with host forwarding, network consoles or host directory sharing"
        )


def _pdeathsig(parent_pid: int) -> None:
    libc = ctypes.CDLL(None, use_errno=True)
    if libc.prctl(PR_SET_PDEATHSIG, signal.SIGKILL, 0, 0, 0) != 0:
        os._exit(127)
    if os.getppid() != parent_pid:  # includes adoption by a living subreaper
        os._exit(127)


def launch(argv: list[str], log: Path) -> subprocess.Popen:
    """Start an owned child that is killed if this harness dies."""
    executable = shutil.which(argv[0])
    if executable is None:
        raise ValueError("owned executable unavailable")
    metadata = Path(executable).stat()
    if not stat.S_ISREG(metadata.st_mode) or metadata.st_mode & (
        stat.S_ISUID | stat.S_ISGID
    ):
        raise ValueError("privileged executable could clear parent-death signal")
    try:
        capabilities = os.getxattr(executable, "security.capability")
    except OSError as error:
        if error.errno not in (errno.ENODATA, errno.ENOTSUP):
            raise
    else:
        if capabilities:
            raise ValueError("file capabilities could clear parent-death signal")
    parent_pid = os.getpid()
    with log.open("ab") as stream:
        return subprocess.Popen(
            argv,
            stdin=subprocess.DEVNULL,
            stdout=stream,
            stderr=subprocess.STDOUT,
            # The launcher is single-threaded; PR_SET_PDEATHSIG needs the child.
            preexec_fn=lambda: _pdeathsig(parent_pid),  # noqa: PLW1509
            close_fds=True,
        )


def start_ticks(pid: int) -> int:
    text = Path(f"/proc/{pid}/stat").read_text()
    return int(text[text.rindex(")") + 2 :].split()[19])


def qmp(sock_path: Path, *commands: str, timeout: float = 5) -> list[dict]:
    replies = []
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as sock:
        sock.settimeout(timeout)
        deadline = time.monotonic() + timeout
        while True:
            try:
                sock.connect(str(sock_path))
                break
            except (FileNotFoundError, ConnectionRefusedError):
                if time.monotonic() > deadline:
                    raise
                time.sleep(0.1)
        stream = sock.makefile("rwb")
        greeting = json.loads(stream.readline())
        replies.append(greeting)
        for command in ("qmp_capabilities", *commands):
            stream.write(json.dumps({"execute": command}).encode() + b"\n")
            stream.flush()
            while True:
                reply = json.loads(stream.readline())
                if "event" in reply:
                    continue
                replies.append(reply)
                break
    return replies


def stop(process: subprocess.Popen, qmp_sock: Path, grace: float = 10) -> list[str]:
    """QMP quit, then SIGKILL; always reap."""
    actions = []
    if process.poll() is None:
        try:
            qmp(qmp_sock, "quit", timeout=3)
            actions.append("qmp-quit")
        except (OSError, ValueError) as error:
            actions.append(f"qmp-quit-failed:{type(error).__name__}")
        try:
            process.wait(grace)
        except subprocess.TimeoutExpired:
            process.kill()
            actions.append("sigkill")
    process.wait()
    return actions


def marker_processes(marker: str) -> list[int]:
    found = []
    for entry in Path("/proc").iterdir():
        if entry.name.isdigit():
            try:
                if marker.encode() in (entry / "cmdline").read_bytes():
                    found.append(int(entry.name))
            except OSError:
                continue
    return found


def _tar_number(field: bytes) -> int:
    """USTAR octal only; no signed/base-256 metadata or extension parser."""
    value = field.strip(b"\0 ")
    if not value or any(c not in b"01234567" for c in value):
        raise ValueError("invalid USTAR integer")
    return int(value, 8)


def _tar_name(field: bytes) -> str:
    name, _, tail = field.partition(b"\0")
    if any(tail):
        raise ValueError("nonzero USTAR string padding")
    return name.decode("ascii")


def _result_header(header: bytes) -> tuple[str, int, bool]:
    if len(header) != 512 or header[257:265] != b"ustar\x0000":
        raise ValueError("results require closed USTAR headers")
    checksum = sum(header[:148]) + 8 * 32 + sum(header[156:])
    if _tar_number(header[148:156]) != checksum:
        raise ValueError("invalid result header checksum")
    if header[156:157] not in (b"0", b"5"):
        raise ValueError("result extensions, links and special files are refused")
    if any(header[157:257]) or any(header[500:]):
        raise ValueError("unexpected result link or extension metadata")
    for field in (header[100:108], header[108:116], header[116:124], header[136:148]):
        _tar_number(field)
    size = _tar_number(header[124:136])
    name, prefix = _tar_name(header[:100]), _tar_name(header[345:500])
    name = f"{prefix}/{name}" if prefix else name
    directory = header[156:157] == b"5"
    name = name.removesuffix("/") if directory else name
    if (
        not name
        or len(name) > 256
        or not re.fullmatch(r"[A-Za-z0-9._/-]+", name)
        or any(part in ("", ".", "..") for part in name.split("/"))
    ):
        raise ValueError(f"unsafe result member name: {name!r}")
    if size > MAX_RESULT_MEMBER or (directory and size != 0):
        raise ValueError("invalid result member size")
    return name, size, directory


def _read_exact(stream, size: int) -> bytes:
    data = stream.read(size)
    if len(data) != size:
        raise ValueError("truncated result archive")
    return data


def validate_done(done: dict, run_id: str) -> dict:
    if (
        not isinstance(done, dict)
        or set(done) != {"run_id", "stages", "seconds"}
        or done["run_id"] != run_id
        or not isinstance(done["stages"], dict)
        or set(done["stages"]) != EXPECTED_STAGES
        or any(
            value not in ("passed", "failed", "skipped")
            for value in done["stages"].values()
        )
        or type(done["seconds"]) not in (int, float)
        or not 0 <= done["seconds"] <= 3600
    ):
        raise ValueError(
            "results completion record differs from this run/stage contract"
        )
    return done["stages"]


def extract_results(image: Path, destination: Path, run_id: str) -> list[str]:
    """Admit fixed headers before payload; publish only complete results."""
    if destination.exists() or destination.is_symlink():
        raise ValueError("results destination must be fresh")
    if image.stat().st_size > RESULTS_BYTES:
        raise ValueError("results disk exceeds its bound")
    destination.parent.mkdir(parents=True, exist_ok=True)
    staging = Path(tempfile.mkdtemp(prefix=".rw040-results-", dir=destination.parent))
    names, seen, total, count = [], set(), 0, 0
    try:
        with image.open("rb") as stream:
            while True:
                header = _read_exact(stream, 512)
                if header == bytes(512):
                    if _read_exact(stream, 512) != bytes(512):
                        raise ValueError("results lack two zero end blocks")
                    # Preallocated disk may contain bounded zero fill only.
                    for chunk in iter(lambda: stream.read(64 << 10), b""):
                        if any(chunk):
                            raise ValueError("nonzero data after result archive")
                    break
                count += 1
                if count > MAX_RESULT_MEMBERS:
                    raise ValueError("too many result members")
                name, size, directory = _result_header(header)
                if name in seen:
                    raise ValueError("duplicate result member")
                seen.add(name)
                total += size
                if total > MAX_RESULT_TOTAL:
                    raise ValueError("results exceed the total bound")
                target = staging / name
                if directory:
                    target.mkdir(parents=True, exist_ok=True)
                    continue
                target.parent.mkdir(parents=True, exist_ok=True)
                with target.open("xb") as output:
                    left = size
                    while left:
                        chunk = _read_exact(stream, min(left, 64 << 10))
                        output.write(chunk)
                        left -= len(chunk)
                if any(_read_exact(stream, (-size) % 512)):
                    raise ValueError("nonzero result member padding")
                names.append(name)
        if "done.json" not in names:
            raise ValueError("results lack done.json")
        done_path = staging / "done.json"
        if done_path.stat().st_size > 8192:
            raise ValueError("completion record too large")
        validate_done(
            json.loads(done_path.read_text(), object_pairs_hook=unique_object), run_id
        )
        staging.rename(destination)
        return names
    finally:
        if staging.exists():
            shutil.rmtree(staging)


def _pidfd_exited(fd: int, timeout: float = 0) -> bool:
    poller = select.poll()
    poller.register(fd, select.POLLIN)
    events = poller.poll(int(timeout * 1000))
    if any(flags & (select.POLLERR | select.POLLNVAL) for _, flags in events):
        raise OSError("invalid process identity descriptor")
    return any(flags & select.POLLIN for _, flags in events)


def _qemu_matches(pid: int, ticks: int, marker: str) -> bool:
    argv = Path(f"/proc/{pid}/cmdline").read_bytes().rstrip(b"\0").split(b"\0")
    return (
        start_ticks(pid) == ticks
        and bool(argv)
        and Path(os.fsdecode(argv[0])).name == "qemu-system-x86_64"
        and any(
            argv[index : index + 2] == [b"-name", f"{marker},process={marker}".encode()]
            for index in range(len(argv) - 1)
        )
    )


def _owner_live(pid: int, ticks: int) -> bool:
    """Inaccessible identity is ambiguous, not permission to remove staging."""
    try:
        fd = os.pidfd_open(pid)
    except ProcessLookupError:
        return False
    try:
        if _pidfd_exited(fd):
            return False
        try:
            return start_ticks(pid) == ticks
        except FileNotFoundError:
            return not _pidfd_exited(fd)
    finally:
        os.close(fd)


def sweep(work_root: Path) -> list[dict]:
    """Signal retained exact identities; preserve staging until observed exit."""
    actions = []
    for run_dir in sorted(work_root.glob("*")) if work_root.exists() else []:
        fd = None
        try:
            if run_dir.is_symlink() or not run_dir.is_dir():
                continue
            owner_file = run_dir / "owner.json"
            if owner_file.is_symlink() or not owner_file.is_file():
                continue
            if owner_file.stat().st_size > 4096:
                raise ValueError("owner record too large")
            owner = json.loads(owner_file.read_text())
            pid, ticks = owner["harness_pid"], owner["harness_start_ticks"]
            if type(pid) is not int or pid <= 0 or type(ticks) is not int or ticks < 0:
                raise ValueError("invalid harness identity")
            if _owner_live(pid, ticks):
                actions.append({"run": run_dir.name, "action": "kept: harness alive"})
                continue
            pid = owner.get("qemu_pid")
            if pid is None:
                actions.append(
                    {"run": run_dir.name, "action": "kept: QEMU identity unavailable"}
                )
                continue
            if pid is not None:
                ticks, marker = owner["qemu_start_ticks"], owner["marker"]
                if (
                    type(pid) is not int
                    or pid <= 0
                    or type(ticks) is not int
                    or ticks < 0
                    or not isinstance(marker, str)
                    or not re.fullmatch(r"rw040-qual-[A-Za-z0-9-]{1,80}", marker)
                ):
                    raise ValueError("invalid QEMU identity")
                try:
                    fd = os.pidfd_open(pid)
                except ProcessLookupError:
                    fd = None
                if fd is not None and not _pidfd_exited(fd):
                    if not _qemu_matches(pid, ticks, marker):
                        actions.append(
                            {"run": run_dir.name, "action": "kept: identity mismatch"}
                        )
                        continue
                    try:
                        signal.pidfd_send_signal(fd, signal.SIGKILL)
                    except ProcessLookupError:
                        if not _pidfd_exited(fd):
                            raise
                    if not _pidfd_exited(fd, timeout=2):
                        actions.append(
                            {"run": run_dir.name, "action": "kept: exit pending"}
                        )
                        continue
                    actions.append(
                        {
                            "run": run_dir.name,
                            "action": f"observed stale qemu exit {pid}",
                        }
                    )
            shutil.rmtree(run_dir)
            actions.append({"run": run_dir.name, "action": "removed"})
        except (OSError, ValueError, KeyError, TypeError) as error:
            actions.append(
                {
                    "run": run_dir.name,
                    "action": f"kept: {type(error).__name__}: {error}",
                }
            )
        finally:
            if fd is not None:
                os.close(fd)
    return actions


def git_head() -> str:
    return subprocess.run(
        ["git", "rev-parse", "HEAD"],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=True,
    ).stdout.strip()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--image",
        type=Path,
        default=ROOT / ".cache/rw040/image/debian-12-genericcloud-amd64.qcow2",
    )
    parser.add_argument(
        "--checksums", type=Path, default=ROOT / ".cache/rw040/image/SHA512SUMS"
    )
    parser.add_argument("--packages-dir", type=Path)
    parser.add_argument(
        "--package-manifest", type=Path, default=FIXTURES / "packages-2fd7420.json"
    )
    parser.add_argument(
        "--derive-packages",
        action="store_true",
        help="Record identities of packages built in this workflow instead of a fixed record",
    )
    parser.add_argument("--install-package", default="limeos_0.4.4_amd64.deb")
    parser.add_argument(
        "--probe",
        type=Path,
        help="Hash-bound unchanged integrator probe supply directory",
    )
    parser.add_argument("--memory-mib", type=int, default=1536)
    parser.add_argument("--cpus", type=int, default=2)
    parser.add_argument("--deadline-seconds", type=int, default=1800)
    parser.add_argument("--budget-total", type=int, default=65)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--work-root", type=Path, default=WORK_ROOT)
    parser.add_argument("--sweep", action="store_true")
    args = parser.parse_args()
    if args.sweep:
        print(json.dumps(sweep(args.work_root), indent=2))
        return 0
    if not args.packages_dir or not args.output:
        parser.error("--packages-dir and --output are required")
    if args.output.exists():
        parser.error("output exists; choose a fresh evidence directory")
    if not (
        1 <= args.budget_total <= 80
        and 512 <= args.memory_mib <= 4096
        and 180 <= args.deadline_seconds <= 3600
    ):
        parser.error("budget, memory or deadline outside the harness bounds")
    started_wall = datetime.datetime.now(datetime.timezone.utc)
    package_manifest = (
        derive_package_manifest(args.packages_dir, git_head())
        if args.derive_packages
        else json.loads(args.package_manifest.read_text())
    )
    image_sha512 = verify_image(args.image, args.checksums)
    packages = verify_packages(args.packages_dir, package_manifest)
    probe_record = (
        verify_probe(args.probe, package_manifest["tested_source"])
        if args.probe
        else None
    )
    install = next(
        (p for p in package_manifest["packages"] if p["file"] == args.install_package),
        None,
    )
    if install is None:
        parser.error(
            f"--install-package {args.install_package} is not among the verified packages"
        )
    layout = json.loads((FIXTURES / "layout.json").read_text())
    run_id = f"{started_wall:%Y%m%dT%H%M%SZ}-{secrets.token_hex(4)}"
    marker = f"rw040-qual-{run_id}"
    guest_budget = args.deadline_seconds - 120
    manifest = {
        "run_id": run_id,
        "source_commit": git_head(),
        "packages": packages,
        "install_package": args.install_package,
        "expected_binaries": install["binaries"],
        "guest_budget_seconds": guest_budget,
        "budget_total_containers": args.budget_total,
        "supplied": ["integrator-probe"] if probe_record else [],
        "probe": probe_record,
        "tested_source": package_manifest["tested_source"],
    }
    run_dir = args.work_root / run_id
    run_dir.mkdir(parents=True)
    args.output.mkdir(parents=True)
    record = {
        "run_id": run_id,
        "marker": marker,
        "started_utc": started_wall.isoformat(),
        "argv": sys.argv,
        "source_commit": manifest["source_commit"],
        "source_dirty": bool(
            subprocess.run(
                [
                    "git",
                    "status",
                    "--porcelain",
                    "--",
                    "tests/qualification/rw040-combined",
                    "tests/fixtures/rw040-combined",
                ],
                cwd=ROOT,
                capture_output=True,
                text=True,
                check=True,
            ).stdout.strip()
        ),
        "image": {"path": args.image.name, "sha512": image_sha512},
        "packages": packages,
        "probe": probe_record,
        "harness_sha256": {
            p.name: sha256(p) for p in [Path(__file__), *(HERE / "guest").glob("*.py")]
        },
        "fixtures_sha256": {n: sha256(FIXTURES / n) for n in FIXTURE_FILES},
        "package_record": "derived"
        if args.derive_packages
        else {
            "path": str(args.package_manifest.relative_to(ROOT))
            if args.package_manifest.is_relative_to(ROOT)
            else str(args.package_manifest),
            "sha256": sha256(args.package_manifest),
        },
        "qemu_version": subprocess.run(
            ["qemu-system-x86_64", "--version"],
            capture_output=True,
            text=True,
            check=True,
        ).stdout.splitlines()[0],
        "host_kernel": os.uname().release,
        "memory_mib": args.memory_mib,
        "cpus": args.cpus,
        "deadline_seconds": args.deadline_seconds,
    }
    owner = {
        "harness_pid": os.getpid(),
        "harness_start_ticks": start_ticks(os.getpid()),
        "marker": marker,
    }
    (run_dir / "owner.json").write_text(json.dumps(owner))
    process = None
    outcome = "failed"
    try:
        inputs_tar(run_dir / "inputs.tar", manifest, args.packages_dir, args.probe)
        record["inputs_tar_sha256"] = sha256(run_dir / "inputs.tar")
        (run_dir / "user-data").write_text(user_data())
        (run_dir / "meta-data").write_text(meta_data(run_id))
        subprocess.run(
            [
                "genisoimage",
                "-quiet",
                "-output",
                str(run_dir / "seed.iso"),
                "-volid",
                "cidata",
                "-joliet",
                "-rock",
                str(run_dir / "user-data"),
                str(run_dir / "meta-data"),
            ],
            check=True,
        )
        subprocess.run(
            [
                "qemu-img",
                "create",
                "-q",
                "-f",
                "qcow2",
                "-F",
                "qcow2",
                "-b",
                str(args.image.resolve()),
                str(run_dir / "system.qcow2"),
                "10G",
            ],
            check=True,
        )
        for index, disk in enumerate(layout["disks"]):
            subprocess.run(
                [
                    "qemu-img",
                    "create",
                    "-q",
                    "-f",
                    "qcow2",
                    str(run_dir / f"data-{index}.qcow2"),
                    f"{disk['size_mib']}M",
                ],
                check=True,
            )
        with (run_dir / "results.img").open("wb") as stream:
            stream.truncate(RESULTS_BYTES)
        argv = qemu_argv(run_dir, marker, layout, args.memory_mib, args.cpus)
        record["qemu_argv"] = [a.replace(str(run_dir), "RUN") for a in argv]
        deadline = time.monotonic() + args.deadline_seconds
        process = launch(argv, run_dir / "qemu.log")
        owner.update(
            {"qemu_pid": process.pid, "qemu_start_ticks": start_ticks(process.pid)}
        )
        (run_dir / "owner.json").write_text(json.dumps(owner))
        launched = time.monotonic()
        record["qmp_initial"] = qmp(run_dir / "qmp.sock", "query-status", timeout=10)[
            -1
        ]
        seen = 0
        while process.poll() is None and time.monotonic() < deadline:
            time.sleep(2)
            text = (run_dir / "console.log").read_bytes().decode(errors="replace")
            for line in text[seen:].splitlines():
                if "[rw040" in line:
                    print(line.strip(), flush=True)
            seen = len(text)
        record["deadline_reached"] = process.poll() is None
        record["teardown_actions"] = stop(process, run_dir / "qmp.sock")
        record["qemu_exit"] = process.returncode
        record["guest_seconds"] = round(time.monotonic() - launched, 1)
        shutil.copy2(run_dir / "console.log", args.output / "console.log")
        shutil.copy2(run_dir / "qemu.log", args.output / "qemu.log")
        try:
            record["result_files"] = extract_results(
                run_dir / "results.img", args.output / "guest", run_id
            )
            done = json.loads((args.output / "guest/done.json").read_text())
            record["guest_stages"] = done["stages"]
            outcome = (
                "completed"
                if all(v == "passed" for v in done["stages"].values())
                else "completed-with-failures"
            )
        except (ValueError, tarfile.TarError, OSError) as error:
            record["results_error"] = f"{type(error).__name__}: {error}"
    finally:
        if process is not None and process.poll() is None:
            record.setdefault("teardown_actions", []).extend(
                stop(process, run_dir / "qmp.sock")
            )
        shutil.rmtree(run_dir, ignore_errors=True)
        record["teardown"] = {
            "run_dir_removed": not run_dir.exists(),
            "marker_processes_after": marker_processes(marker),
            "finished_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        }
        record["outcome"] = outcome
        (args.output / "run.json").write_text(
            json.dumps(record, indent=2, sort_keys=True) + "\n"
        )
        print(
            json.dumps(
                {"outcome": outcome, "run_id": run_id, "teardown": record["teardown"]}
            )
        )
    return 0 if outcome == "completed" else 1


if __name__ == "__main__":
    sys.exit(main())
