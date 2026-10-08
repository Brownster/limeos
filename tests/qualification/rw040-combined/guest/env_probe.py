#!/usr/bin/env python3
"""Record what the current credentials and confinement permit, with stdlib only.

Environment baseline, not library qualification: it uses the same kernel
interfaces the RW-040 libraries need (Docker socket and peer credentials,
pidfd, openat2, statx, proc namespace/root links) so a later probe outcome can
be explained. Run once as unrestricted guest root and once under the
reproduced storage-reader confinement. Prints one JSON document.
"""

import argparse
import ctypes
import errno
import json
import os
import resource
import socket
import struct
import time
from pathlib import Path

LIBC = ctypes.CDLL(None, use_errno=True)
SYS_OPENAT2 = 437
SYS_STATX = 332
RESOLVE_NO_MAGICLINKS = 0x02
RESOLVE_NO_SYMLINKS = 0x04
O_PATH = 0o10000000
O_DIRECTORY = 0o200000
O_CLOEXEC = 0o2000000
AT_FDCWD = -100
STATX_BASIC_STATS = 0x7FF
STATX_MNT_ID = 0x1000


def outcome(action):
    try:
        value = action()
        return {"ok": True, "value": value}
    except OSError as error:
        code = error.errno
        return {
            "ok": False,
            "errno": code,
            "name": errno.errorcode.get(code, str(code)),
        }
    except Exception as error:  # noqa: BLE001 - recorded, not hidden
        return {"ok": False, "error": type(error).__name__, "detail": str(error)[:200]}


def syscall(number, *args):
    LIBC.syscall.restype = ctypes.c_long
    result = LIBC.syscall(number, *args)
    if result < 0:
        code = ctypes.get_errno()
        raise OSError(code, os.strerror(code))
    return result


def openat2(path):
    how = struct.pack(
        "QQQ",
        O_PATH | O_DIRECTORY | O_CLOEXEC,
        0,
        RESOLVE_NO_SYMLINKS | RESOLVE_NO_MAGICLINKS,
    )
    buffer = ctypes.create_string_buffer(how)
    fd = syscall(
        SYS_OPENAT2,
        ctypes.c_int(AT_FDCWD),
        ctypes.c_char_p(path.encode()),
        buffer,
        ctypes.c_size_t(len(how)),
    )
    os.close(fd)
    return "opened"


def statx(path):
    buffer = ctypes.create_string_buffer(256)
    syscall(
        SYS_STATX,
        ctypes.c_int(AT_FDCWD),
        ctypes.c_char_p(path.encode()),
        ctypes.c_int(0),
        ctypes.c_uint(STATX_BASIC_STATS | STATX_MNT_ID),
        buffer,
    )
    mask = struct.unpack_from("I", buffer.raw, 0)[0]
    mnt_id = (
        struct.unpack_from("Q", buffer.raw, 144)[0] if mask & STATX_MNT_ID else None
    )
    return {
        "mask": mask,
        "mnt_id_reported": bool(mask & STATX_MNT_ID),
        "mnt_id": mnt_id,
    }


def status_fields():
    keep = (
        "Uid",
        "Gid",
        "Groups",
        "CapInh",
        "CapPrm",
        "CapEff",
        "CapBnd",
        "CapAmb",
        "NoNewPrivs",
        "Seccomp",
    )
    with open("/proc/self/status") as stream:
        return {
            k: v.strip()
            for k, v in (line.split(":", 1) for line in stream)
            if k in keep
        }


def cgroup_limits():
    with open("/proc/self/cgroup") as stream:
        path = stream.read().strip().split("::", 1)[-1]
    out = {"cgroup": path}
    for name in ("memory.max", "memory.swap.max", "pids.max", "cpu.max"):
        out[name] = outcome(
            lambda n=name: Path(f"/sys/fs/cgroup{path}/{n}").read_text().strip()
        )
    return out


def docker(path):
    def run():
        sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        sock.settimeout(5)
        try:
            sock.connect(path)
            pid, uid, gid = struct.unpack(
                "3i", sock.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, 12)
            )
            sock.sendall(b"GET /_ping HTTP/1.0\r\nHost: docker\r\n\r\n")
            reply = sock.recv(256).split(b"\r\n", 1)[0].decode(errors="replace")
            return {
                "peer_pid": pid,
                "peer_uid": uid,
                "peer_gid": gid,
                "status_line": reply,
            }
        finally:
            sock.close()

    return outcome(run)


def target(pid):
    base = f"/proc/{pid}"
    return {
        "pid": pid,
        "status_owner_uid": outcome(lambda: os.stat(f"{base}/status").st_uid),
        "read_status": outcome(lambda: len(open(f"{base}/status").read())),
        "pidfd_open": outcome(lambda: os.close(os.pidfd_open(pid)) or "opened"),
        "readlink_root": outcome(lambda: os.readlink(f"{base}/root")),
        "open_root": outcome(
            lambda: (
                os.close(os.open(f"{base}/root", os.O_PATH | os.O_DIRECTORY))
                or "opened"
            )
        ),
        "stat_root": outcome(lambda: list(os.stat(f"{base}/root/")[1:3])),
        "readlink_ns_mnt": outcome(lambda: os.readlink(f"{base}/ns/mnt")),
        "open_ns_mnt": outcome(
            lambda: os.close(os.open(f"{base}/ns/mnt", os.O_RDONLY)) or "opened"
        ),
        "read_mountinfo": outcome(lambda: len(open(f"{base}/mountinfo").read())),
        "statx_root": outcome(lambda: statx(f"{base}/root/")),
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--targets", required=True, help="JSON object: label -> PID")
    parser.add_argument("--docker-socket", default="/run/docker.sock")
    args = parser.parse_args()
    targets = json.loads(args.targets)
    proc_options = next(
        (
            line.split()[5] + " " + line.split(" - ")[1].split()[2]
            for line in Path("/proc/self/mountinfo").read_text().splitlines()
            if line.split()[4] == "/proc"
        ),
        None,
    )
    report = {
        "time": time.time(),
        "euid": os.geteuid(),
        "status": status_fields(),
        "rlimit_nofile": resource.getrlimit(resource.RLIMIT_NOFILE),
        "cgroup": cgroup_limits(),
        "proc_mount_options": proc_options,
        "openat2_proc": outcome(lambda: openat2("/proc")),
        "openat2_var_run_docker": outcome(lambda: openat2("/var/run")),
        "statx_root": outcome(lambda: statx("/")),
        "inet_socket": outcome(
            lambda: (
                socket.socket(socket.AF_INET, socket.SOCK_STREAM).close() or "created"
            )
        ),
        "docker_canonical": docker(args.docker_socket),
        "docker_var_run": docker("/var/run/docker.sock"),
        "pid1": target(1),
        "targets": {label: target(pid) for label, pid in targets.items()},
    }
    print(json.dumps(report, sort_keys=True))


if __name__ == "__main__":
    main()
