"""Bounded disposable-guest setup observations, never production admission."""

import http.client
import json
import os
import socket
import struct
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


def _start(proc):
    raw = _read(proc / "stat")
    return int(raw[raw.rindex(")") + 1 :].split()[19])


def _ids(path, limit):
    result = []
    with os.scandir(path) as entries:
        for entry in entries:
            name = entry.name
            if (
                len(result) >= limit
                or not name.isascii()
                or not name.isdecimal()
                or len(name) > 10
                or int(name) > 0xFFFF_FFFF
            ):
                raise ValueError("Invalid fixture Docker proc membership")
            result.append(int(name))
    return sorted(result)


def _docker_membership(pid, start):
    proc = PROC / str(pid)
    if _start(proc) != start:
        raise ValueError("Fixture Docker parent changed")
    tasks = [(tid, _start(proc / "task" / str(tid))) for tid in _ids(proc / "task", 64)]
    fds = _ids(proc / "fd", 256)
    if _start(proc) != start:
        raise ValueError("Fixture Docker parent changed")
    return {"tasks": tasks, "fds": fds}


def _docker_json(peer, path, limit, deadline):
    # These fixed GETs exercise the real zero-container collector's HTTP paths.
    # No daemon configuration, plan, approval or effect endpoint is called.
    connection = http.client.HTTPConnection("localhost", timeout=_before(deadline))
    try:
        connection.sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        connection.sock.settimeout(_before(deadline))
        connection.sock.connect("/run/docker.sock")
        actual = struct.unpack(
            "3i", connection.sock.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, 12)
        )
        if actual != (peer["pid"], peer["uid"], peer["gid"]):
            raise ValueError("Fixture Docker peer changed")
        connection.request("GET", path, headers={"Connection": "close"})
        reply = connection.getresponse()
        raw = reply.read(limit + 1)
        _before(deadline)
        if reply.status != 200 or len(raw) > limit:
            raise ValueError("Invalid fixture Docker read response")
        return json.loads(raw)
    finally:
        connection.close()


def wait_docker_startup(peer, engine_id):
    """Exercise read paths, then observe natural startup settling before planning."""
    _guest()
    if (
        type(peer.get("pid")) is not int
        or not 1 < peer["pid"] <= 0xFFFF_FFFF
        or peer.get("uid") != 0
        or not isinstance(engine_id, str)
        or not 1 <= len(engine_id) <= 256
    ):
        raise ValueError("Live fixture Docker identity required")
    started = time.monotonic()
    deadline = started + 15
    start = _start(PROC / str(peer["pid"]))
    initial = previous = _docker_membership(peer["pid"], start)
    changes = 0
    for _ in range(4):
        _before(deadline)
        version = _docker_json(peer, "/version", 8192, deadline)
        api = version.get("ApiVersion") if isinstance(version, dict) else None
        minimum = (
            version.get("MinAPIVersion", "1.24") if isinstance(version, dict) else None
        )
        if not all(
            isinstance(v, str)
            and v.startswith("1.")
            and v[2:].isascii()
            and v[2:].isdecimal()
            for v in (api, minimum)
        ):
            raise ValueError("Invalid fixture Docker API version")
        maximum = min(int(api[2:]), 56)
        if maximum < 41 or int(minimum[2:]) > maximum:
            raise ValueError("Unsupported fixture Docker API version")
        prefix = f"/v1.{maximum}"
        info = _docker_json(peer, prefix + "/info", 128 * 1024, deadline)
        containers = _docker_json(
            peer, prefix + "/containers/json?all=1", 512 * 1024, deadline
        )
        if (
            not isinstance(info, dict)
            or info.get("ID") != engine_id
            or containers != []
        ):
            raise ValueError("Fixture Docker identity or empty inventory changed")
    # Warmup connections themselves change daemon FDs. Close all of them before
    # quiet observation; no network requests or authority acquisition occur here.
    quiet_since = time.monotonic()
    for attempt in range(96):
        _before(deadline)
        current = _docker_membership(peer["pid"], start)
        now = time.monotonic()
        _before(deadline)
        if current != previous:
            changes += 1
            quiet_since = now
        if now - quiet_since >= 3:
            return {
                "scope": "read-only fixture startup; no admission or later runtime authority",
                "parent_pid": peer["pid"],
                "parent_start": start,
                "initial": initial,
                "final": current,
                "membership_changes": changes,
                "read_passes": 4,
                "quiet_observations": attempt + 1,
                "quiet_seconds": now - quiet_since,
                "elapsed_seconds": now - started,
                "service_stopped_or_modified": False,
            }
        previous = current
        time.sleep(min(0.1, _before(deadline)))
    raise TimeoutError(
        f"Fixture Docker startup attempt limit exceeded; changes={changes}, "
        f"quiet_seconds={time.monotonic() - quiet_since:.3f}, "
        f"elapsed_seconds={time.monotonic() - started:.3f}"
    )


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
