"""Bounded disposable-guest setup observations, never production admission."""

import json
import os
import socket
import time
from pathlib import Path

PROC = Path("/proc")
READER_SOCKET = Path("/run/limeos-storage-reader/executor.sock")


def _guest():
    if os.geteuid() != 0 or socket.gethostname() != "limeos-p01-test":
        raise ValueError("Disposable guest required for fixture readiness")


def _before(deadline):
    remaining = deadline - time.monotonic()
    if remaining <= 0:
        raise TimeoutError("Fixture readiness deadline expired")
    return remaining


def wait_storage_reader(run, probe_code):
    """Observe successful Core-UID inventory before later strict API assertions."""
    _guest()
    started = time.monotonic()
    deadline = started + 6
    observations = []
    while not READER_SOCKET.is_socket():
        _before(deadline)
        time.sleep(0.05)
    for _ in range(64):
        reply = run(
            "runuser",
            "-u",
            "limeos-core",
            "-g",
            "limeos-host-access",
            "--",
            "python3",
            "-c",
            probe_code,
            str(READER_SOCKET),
            json.dumps({"operation": "storage_inventory", "version": 1}),
            timeout=_before(deadline),
        )
        _before(deadline)
        if len(reply.stdout.encode()) > 65536:
            raise ValueError("Oversized fixture inventory response")
        value = json.loads(reply.stdout)
        if not isinstance(value, dict):
            raise TypeError("Malformed fixture inventory response")
        ready = (
            type(value.get("version")) is int
            and value["version"] == 1
            and value.get("ready") is True
            and value.get("error") is None
            and isinstance(value.get("storage"), dict)
        )
        observations.append({"ready": ready, "error": value.get("error")})
        if ready:
            return {
                "scope": "read-only fixture startup; no effect or admission authority",
                "elapsed_seconds": time.monotonic() - started,
                "observations": observations,
            }
        if value != {"version": 1, "ready": False, "error": "overloaded"}:
            raise ValueError("Unexpected fixture inventory refusal")
        time.sleep(0.05)
    raise TimeoutError("Fixture inventory attempt limit exceeded")


def _read(path):
    with path.open("rb") as stream:
        value = stream.read(4097)
    if len(value) > 4096:
        raise ValueError("Fixture proc field exceeds bound")
    return value.decode("ascii")


def wait_udev_workers(run):
    """Let this guest's disk-setup workers exit before its first planning RPC."""
    _guest()
    run("udevadm", "settle", "--timeout=10", timeout=10)
    pid = int(
        run(
            "systemctl", "show", "systemd-udevd", "-p", "MainPID", "--value", timeout=5
        ).stdout
    )
    if not 1 < pid <= 0xFFFF_FFFF:
        raise ValueError("Live fixture udev parent required")
    proc = PROC / str(pid)

    def start():
        raw = _read(proc / "stat")
        fields = raw[raw.rindex(")") + 1 :].split()
        return int(fields[19])

    def children():
        fields = _read(proc / "task" / str(pid) / "children").split()
        if len(fields) > 64 or any(
            not field.isascii()
            or not field.isdecimal()
            or len(field) > 10
            or not 0 < int(field) <= 0xFFFF_FFFF
            for field in fields
        ):
            raise ValueError("Invalid fixture udev child membership")
        return [int(field) for field in fields]

    identity = start()
    initial = children()
    started = time.monotonic()
    pending = initial
    while pending:
        _before(started + 35)
        if start() != identity:
            raise ValueError("Fixture udev parent changed")
        time.sleep(0.1)
        pending = children()
    if start() != identity:
        raise ValueError("Fixture udev parent changed")
    _before(started + 35)
    return {
        "scope": "fixture disk-setup quiescence; not admission or later runtime evidence",
        "parent_pid": pid,
        "parent_start": identity,
        "initial_children": initial,
        "final_children": pending,
        "elapsed_seconds": time.monotonic() - started,
        "service_stopped_or_modified": False,
    }
