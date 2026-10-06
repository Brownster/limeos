#!/usr/bin/env python3
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Fresh container storage reads after installed lifecycle/crash regressions."""

import hashlib
import json
import os
import socket
import sqlite3
import sys
import threading
from pathlib import Path

import p03_guest as fixture
import p03_lifecycle_guest as lifecycle

BASE = "/api/v1/storage/container-dependencies"


class ReadFaults:
    def __init__(self):
        self.mode = ""
        self.target = ""
        self.calls = []
        self.counts = {}
        self.entered = threading.Event()
        self.release = threading.Event()
        self.lock = threading.Lock()

    def arm(self, mode, target=""):
        self.mode, self.target = mode, target
        self.counts.clear()
        self.entered.clear()
        self.release.clear()


faults = ReadFaults()
original_get = fixture.Proxy.do_GET


def dependency_get(handler):
    with faults.lock:
        faults.calls.append(handler.path)
        count = faults.counts.get(handler.path, 0) + 1
        faults.counts[handler.path] = count
    info = handler.path.endswith("/info")
    inspect = faults.target in handler.path and handler.path.endswith("/json")
    body = None
    code = 200
    if faults.mode == "missing_info" and info:
        body = {"ID": None}
    elif faults.mode == "oversized_info" and info:
        body = {"ID": "synthetic-engine", "padding": "x" * (128 * 1024)}
    elif faults.mode == "missing_inspection" and inspect:
        body, code = {"message": "synthetic unavailable"}, 404
    elif (
        faults.mode == "change_list"
        and "containers/json?all=1" in handler.path
        and count == 2
    ):
        body = []
    elif faults.mode == "change_container" and inspect and count == 2:
        fixture.docker("start", faults.target)
    elif faults.mode == "authority_barrier" and info and count == 1:
        faults.entered.set()
        if not faults.release.wait(5):
            raise AssertionError("Synthetic read authority barrier timed out")
    if body is None:
        original_get(handler)
        return
    data = json.dumps(body).encode()
    try:
        handler.send_response(code)
        handler.send_header("Content-Length", str(len(data)))
        handler.end_headers()
        handler.wfile.write(data)
    except (BrokenPipeError, ConnectionResetError):
        pass


def authority_snapshot():
    with sqlite3.connect(fixture.CORE_DB) as db:
        return {
            "jobs": db.execute(
                "SELECT id,intent,digest,state FROM jobs ORDER BY id"
            ).fetchall(),
            "events": db.execute(
                "SELECT cursor,kind FROM events ORDER BY cursor"
            ).fetchall(),
            "locks": db.execute(
                "SELECT resource,job FROM resource_locks ORDER BY resource"
            ).fetchall(),
        }


def storage_grant(principal, enabled, database=fixture.CORE_DB):
    with sqlite3.connect(database) as db:
        if enabled:
            db.execute(
                "INSERT OR IGNORE INTO grants VALUES(?,?,?)",
                (principal, '"storage_manage"', "storage:configuration"),
            )
        else:
            db.execute(
                "DELETE FROM grants WHERE principal=? AND operation=?",
                (principal, '"storage_manage"'),
            )


def main(package_version="0.4.5"):
    if os.getuid() != 0 or socket.gethostname() != "limeos-p01-test":
        raise SystemExit("Disposable guest required")
    lifecycle.main(
        package_version=package_version, authority_schema=8, qualify_upgrade=False
    )
    output = Path(sys.argv[2])
    fixture.Proxy.do_GET = dependency_get
    cookie, _, principal = fixture.enroll()
    root = Path("/mnt/limeos-dependency-fixture/Media")
    root.mkdir(parents=True)
    (root / "sentinel").write_text("synthetic data remains intact\n")
    running = fixture.docker(
        "run",
        "-d",
        "--mount",
        f"type=bind,source={root},target=/data,readonly",
        "--tmpfs",
        "/tmp",
        "--env",
        "DEPENDENCY_SECRET=synthetic-private",
        "--label",
        "private=synthetic-private",
        "limeos-p03-fixture:local",
        "/bin/busybox",
        "sleep",
        "3600",
    )
    stopped = fixture.docker(
        "create",
        "--mount",
        "type=volume,source=dependency-config,target=/config",
        "limeos-p03-fixture:local",
        "/bin/busybox",
        "sleep",
        "3600",
    )
    assert not fixture.inspect(stopped)["State"]["Running"]
    policy_path = Path("/etc/limeos/system-policy/container.json")
    policy = json.loads(policy_path.read_text())
    assert (
        running not in policy["managed_containers"]
        and stopped not in policy["managed_containers"]
    )
    assert fixture.http(BASE)[0] == 401
    storage_grant(principal["id"], False)
    before_calls = len(faults.calls)
    assert fixture.http(BASE, cookie=cookie)[0] == 403
    assert len(faults.calls) == before_calls
    storage_grant(principal["id"], True)
    assert fixture.http(BASE + "?paths=/mnt", cookie=cookie)[0] == 400
    assert fixture.http(BASE, "POST", {}, cookie=cookie)[0] == 405
    fixture.passed(
        "private dependency reads require a human storage grant and accept no filters or mutations"
    )

    before = authority_snapshot()
    posts = dict(fixture.control.posts)
    status, headers, view = fixture.http(BASE, cookie=cookie)
    assert status == 200, view
    assert headers["cache-control"] == "no-store"
    canonical = json.dumps(
        view["inventory"], separators=(",", ":"), ensure_ascii=False
    ).encode()
    assert hashlib.sha256(canonical).hexdigest() == view["digest"]
    by_id = {
        item["container"]["resource"]: item for item in view["inventory"]["containers"]
    }
    expected_ids = {
        "container:" + identifier
        for identifier in fixture.docker("ps", "-aq", "--no-trunc").splitlines()
    }
    assert set(by_id) == expected_ids
    bind = by_id["container:" + running]
    assert bind["container"]["running"] and bind["mounts"] == [
        {
            "source": {"kind": "bind", "path": str(root)},
            "destination": "/data",
            "writable": False,
        }
    ]
    volume = by_id["container:" + stopped]
    assert not volume["container"]["running"]
    actual = fixture.inspect(stopped)["Mounts"][0]
    assert volume["mounts"] == [
        {
            "source": {
                "kind": "volume",
                "path": actual["Source"],
                "name": actual["Name"],
                "driver": actual["Driver"],
            },
            "destination": "/config",
            "writable": actual["RW"],
        }
    ]
    encoded = json.dumps(view)
    assert "DEPENDENCY_SECRET" not in encoded and "synthetic-private" not in encoded
    assert authority_snapshot() == before and dict(fixture.control.posts) == posts
    assert (root / "sentinel").read_text() == "synthetic data remains intact\n"
    fixture.passed(
        "real running unmanaged bind, validated tmpfs and never-started named volume declarations match Engine without leaking secrets"
    )
    fixture.passed(
        "GET records no intent, event, claim or Engine effect and preserves synthetic data"
    )

    for mode in ["missing_info", "oversized_info", "missing_inspection", "change_list"]:
        faults.arm(mode, running)
        status, _, error = fixture.http(BASE, cookie=cookie)
        assert status == (409 if mode == "change_list" else 503), (mode, error)
        assert "inventory" not in error
        faults.mode = ""
        fixture.passed(
            f"live {mode} evidence refuses instead of returning empty or clipped consumers"
        )
    faults.arm("change_container", stopped)
    assert fixture.http(BASE, cookie=cookie)[0] == 409
    faults.mode = ""
    fixture.docker("stop", "-t", "1", stopped)
    fixture.passed(
        "real container start between inspections invalidates the dependency read"
    )

    faults.arm("authority_barrier")
    responses = []
    reader = threading.Thread(
        target=lambda: responses.append(fixture.http(BASE, cookie=cookie))
    )
    reader.start()
    assert faults.entered.wait(3)
    storage_grant(principal["id"], False)
    faults.release.set()
    reader.join(10)
    faults.mode = ""
    assert not reader.is_alive() and responses[0][0] == 403, responses
    storage_grant(principal["id"], True)
    fixture.passed(
        "storage grant revocation during a real read prevents returning private paths"
    )

    original = dict(policy)
    policy.update(
        allow_container_read=False,
        allow_restart=False,
        allow_start=False,
        allow_stop=False,
        allow_container_logs=False,
    )
    policy_path.write_text(json.dumps(policy))
    fixture.restart_container()
    assert fixture.http(BASE, cookie=cookie)[0] != 200
    policy_path.write_text(json.dumps(original))
    fixture.restart_container()
    assert fixture.http(BASE, cookie=cookie)[0] == 200
    fixture.passed(
        "independent container read ceiling disables live dependency enumeration"
    )

    fixture.run("apt-get", "install", "-y", "limeos-shadow=" + package_version)
    shadow_policy_path = Path("/etc/limeos-shadow/system-policy/container.json")
    shadow_policy = json.loads(shadow_policy_path.read_text())
    shadow_policy.update(
        allow_restart=False,
        allow_start=False,
        allow_stop=False,
        allow_container_logs=False,
        allow_container_read=True,
        managed_containers=[],
    )
    shadow_policy_path.write_text(json.dumps(shadow_policy))
    fixture.run("systemctl", "reset-failed", "limeos-shadow-containerd")
    fixture.run("systemctl", "restart", "limeos-shadow-containerd")
    shadow_cookie, _, shadow_principal = fixture.enroll("limeos-shadow", 8004)
    storage_grant(
        shadow_principal["id"], True, "/var/lib/limeos-shadow/core/core.sqlite"
    )
    assert fixture.http(BASE, cookie=shadow_cookie, port=8004)[0] == 200
    standard_pid = fixture.run(
        "systemctl", "show", "limeos-core", "-p", "MainPID", "--value"
    ).stdout
    fixture.run("apt-get", "remove", "-y", "limeos-shadow")
    assert (
        fixture.run(
            "systemctl", "show", "limeos-core", "-p", "MainPID", "--value"
        ).stdout
        == standard_pid
    )
    fixture.passed(
        "isolated shadow reads the live declarations and removal preserves standard services"
    )

    result = json.loads(output.read_text())
    result.update(
        package_version=package_version,
        checks=fixture.checks,
        dependency_inventory=view,
        dependency_cases=11,
        binary_sha256={
            name: hashlib.sha256(
                (Path("/usr/lib/limeos") / name).read_bytes()
            ).hexdigest()
            for name in [
                "limeos-core",
                "limeos-executor",
                "limeosctl",
                "limeos-password-worker",
            ]
        },
        limitations=[
            "Fresh AMD64 disposable guest and synthetic data; no genuine historical upgrade in this run",
            "Docker declarations do not establish physical alias, pool, protection or share completeness; mount/fstab effects remain gated",
        ],
    )
    output.write_text(json.dumps(result, indent=2) + "\n")


if __name__ == "__main__":
    main()
