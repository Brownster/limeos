"""Fixed, read-only socket lookup; only under Core UID in a disposable AMD64 VM."""

import ctypes
import json
import os
import platform
import pwd
import resource
import signal
from pathlib import Path

SOCKET = "/run/limeos-storage-reader/executor.sock"


class OpenHow(ctypes.Structure):
    _fields_ = [
        ("flags", ctypes.c_uint64),
        ("mode", ctypes.c_uint64),
        ("resolve", ctypes.c_uint64),
    ]


def guard():
    if (
        Path("/etc/hostname").read_text().strip() != "limeos-p01-test"
        or platform.machine() != "x86_64"
    ):
        raise SystemExit("Disposable AMD64 guest required")
    if os.geteuid() != pwd.getpwnam("limeos-core").pw_uid:
        raise SystemExit("Actual Core UID required")


def lookup():
    libc = ctypes.CDLL(None, use_errno=True)
    libc.syscall.restype = ctypes.c_long
    how = OpenHow(os.O_PATH | os.O_NOFOLLOW | os.O_CLOEXEC, 0, 0x02 | 0x04)
    ctypes.set_errno(0)
    fd = libc.syscall(
        ctypes.c_long(437),
        ctypes.c_int(-100),
        ctypes.c_char_p(SOCKET.encode()),
        ctypes.byref(how),
        ctypes.c_size_t(ctypes.sizeof(how)),
    )
    result = {"openat2_errno": ctypes.get_errno() if fd < 0 else 0}
    if fd >= 0:
        try:
            meta = os.fstat(fd)
            result["socket"] = {
                "device": meta.st_dev,
                "inode": meta.st_ino,
                "uid": meta.st_uid,
                "gid": meta.st_gid,
                "mode": meta.st_mode,
                "links": meta.st_nlink,
            }
        finally:
            os.close(fd)
    return result


def main():
    guard()
    signal.alarm(3)
    result = lookup()
    text = Path("/proc/self/status").read_text()
    if len(text.encode()) > 16384:
        raise ValueError("process status exceeds bound")
    keys = ("CapEff", "CapPrm", "CapBnd", "NoNewPrivs", "Seccomp", "Threads")
    result.update(
        uid=os.geteuid(),
        gid=os.getegid(),
        groups=os.getgroups(),
        pid=os.getpid(),
        status={
            key: value.strip()
            for line in text.splitlines()
            if ":" in line
            for key, value in [line.split(":", 1)]
            if key in keys
        },
        nofile=list(resource.getrlimit(resource.RLIMIT_NOFILE)),
    )
    encoded = json.dumps(result, sort_keys=True)
    if len(encoded.encode()) > 16384:
        raise ValueError("probe report exceeds bound")
    print("B1_CORE_LOOKUP " + encoded, flush=True)


if __name__ == "__main__":
    main()
