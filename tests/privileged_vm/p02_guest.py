#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""P02 coexistence and read-only acceptance, only in the disposable Debian VM."""

import hashlib
import http.client
import json
import os
import pwd
import socket
import sqlite3
import struct
import subprocess
import sys
import threading
import time
from collections import Counter
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from socketserver import UnixStreamServer


def run(*args, input=None, check=True):
    result = subprocess.run(
        args, input=input, text=True, capture_output=True, check=False
    )
    if check and result.returncode:
        raise RuntimeError(f"{args[0]} failed: {result.stderr[-3000:]}")
    return result


def http_request(port, path, method="GET", body=None, cookie=None):
    connection = http.client.HTTPConnection("127.0.0.1", port, timeout=12)
    headers = {
        "Origin": "https://limeos-shadow.localhost:8444"
        if port == 8004
        else "https://localhost",
        "Content-Type": "application/json",
    }
    if cookie:
        headers["Cookie"] = cookie
    connection.request(method, path, json.dumps(body) if body else None, headers)
    response = connection.getresponse()
    data = response.read()
    result = (
        response.status,
        dict(response.getheaders()),
        json.loads(data) if data else None,
    )
    connection.close()
    return result


def rpc(path, request):
    data = json.dumps(request).encode()
    with socket.socket(socket.AF_UNIX) as stream:
        stream.settimeout(5)
        stream.connect(path)
        stream.sendall(struct.pack("!I", len(data)) + data)
        header = stream.recv(4)
        if len(header) != 4:
            return None
        size = struct.unpack("!I", header)[0]
        body = b""
        while len(body) < size:
            part = stream.recv(size - len(body))
            if not part:
                return None
            body += part
        return json.loads(body)


def enroll(shadow):
    prefix = "limeos-shadow" if shadow else "limeos"
    ctl = [f"/usr/lib/{prefix}/limeosctl", "--socket", f"/run/{prefix}-core/core.sock"]
    token = json.loads(run(*ctl, "bootstrap").stdout)["token"]
    run(
        *ctl,
        "enroll",
        input=json.dumps(
            {
                "token": token,
                "username": "test-admin",
                "password": "Synthetic-VM-password-2026",
            }
        ),
    )
    status, headers, _ = http_request(
        8004 if shadow else 8003,
        "/api/v1/auth/login",
        "POST",
        {"username": "test-admin", "password": "Synthetic-VM-password-2026"},
    )
    assert status == 200
    return headers["set-cookie"].split(";")[0]


def hashes(path):
    return {
        str(p): hashlib.sha256(p.read_bytes()).hexdigest()
        for p in sorted(path.rglob("*"))
        if p.is_file()
    }


def authority_snapshot():
    # Readers update SQLite's WAL index, and normal telemetry has its own writes.
    # Compare durable authority contents rather than disposable physical files.
    with sqlite3.connect(
        "file:/var/lib/limeos/core/core.sqlite?mode=ro", uri=True
    ) as connection:
        return hashlib.sha256("\n".join(connection.iterdump()).encode()).hexdigest()


def pss(pid):
    return sum(
        int(line.split()[1])
        for line in Path(f"/proc/{pid}/smaps_rollup").read_text().splitlines()
        if line.startswith("Pss:")
    )


def main():
    if (
        os.geteuid() != 0
        or Path("/etc/hostname").read_text().strip() != "limeos-p01-test"
    ):
        raise SystemExit("Refusing outside disposable VM")
    repo = Path(sys.argv[1])
    output = Path(sys.argv[2])
    results = []

    def passed(name):
        results.append(name)
        print("PASS " + name, flush=True)

    run(
        "install",
        "-m",
        "0644",
        str(repo / "limeos-archive-keyring.gpg"),
        "/usr/share/keyrings/limeos-test.gpg",
    )
    Path("/etc/apt/sources.list.d/limeos-test.list").write_text(
        f"deb [signed-by=/usr/share/keyrings/limeos-test.gpg] file:{repo} stable main\n"
    )
    run("apt-get", "update")
    run("apt-get", "install", "-y", "limeos=0.2.0")
    baseline_cookie = enroll(False)
    reference = Path("/home/reference/pi-health")
    reference.mkdir(parents=True)
    (reference / "config.json").write_text('{"unchanged":"reference"}\n')
    original = hashes(reference)
    original_state = authority_snapshot()
    units = ["limeos-core", "limeos-containerd", "limeos-storaged"]
    original_pids = {
        u: run("systemctl", "show", u, "-p", "MainPID", "--value").stdout.strip()
        for u in units
    }
    mounts = Path("/proc/1/mountinfo").read_text()
    run("apt-get", "install", "-y", "limeos-shadow=0.2.0")
    shadow_units = [
        "limeos-shadow-core",
        "limeos-shadow-containerd",
        "limeos-shadow-storaged",
    ]
    for unit in shadow_units:
        run("systemctl", "is-active", unit)
    assert original_pids == {
        u: run("systemctl", "show", u, "-p", "MainPID", "--value").stdout.strip()
        for u in units
    }
    assert original == hashes(reference) and original_state == authority_snapshot()
    assert mounts == Path("/proc/1/mountinfo").read_text()
    assert http_request(8004, "/api/v1/overview", cookie=baseline_cookie)[0] == 401
    shadow_cookie = enroll(True)
    assert http_request(8003, "/api/v1/overview", cookie=shadow_cookie)[0] == 401
    passed(
        "signed apt coexistence, independent login/port/state, unchanged existing services and mounts"
    )
    for account in ["limeos-shadow-core", "limeos-shadow-assistant"]:
        assert "docker" not in run("id", "-nG", account).stdout.split()
        assert (
            run(
                "runuser",
                "-u",
                account,
                "--",
                "cat",
                "/var/lib/limeos/core/core.sqlite",
                check=False,
            ).returncode
            != 0
        )
    for target in ["host", "container"]:
        policy = json.loads(
            Path(f"/etc/limeos-shadow/system-policy/{target}.json").read_text()
        )
        assert policy["core_uid"] == pwd.getpwnam("limeos-shadow-core").pw_uid
        assert policy["allow_host_read"] == (target == "host") and policy[
            "allow_container_read"
        ] == (target == "container")
    passed("separate accounts and read-only independent ceilings")
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline:
        status, _, overview = http_request(
            8004, "/api/v1/overview", cookie=shadow_cookie
        )
        if (
            status == 200
            and overview["host"]
            and any(r["kind"] == "disk" for r in overview["resources"])
        ):
            break
        time.sleep(0.2)
    else:
        raise RuntimeError("host observations not ready")
    assert overview["host"]["memory_percent"] is not None
    assert any(
        s["source"] == "docker" and s["state"] == "unavailable"
        for s in overview["sources"]
    )
    assert http_request(8004, "/api/v1/system/history?range=30d", cookie=shadow_cookie)[
        2
    ]["legacy_history"]
    assert (
        http_request(8004, "/api/v1/resources?limit=51", cookie=shadow_cookie)[0] == 400
    )
    passed("real host/disk observations and independent absent Docker source/history")
    # Local fake Engine proves the real daemon's HTTP adapter stays GET-only.
    calls = Counter()
    container_id = "a" * 64

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass

        def do_GET(self):
            calls[self.path.split("?")[0]] += 1
            if self.path == "/version":
                value = {"ApiVersion": "1.41", "MinAPIVersion": "1.24"}
            elif "/containers/json" in self.path:
                value = [
                    {
                        "Id": container_id,
                        "Names": ["/media"],
                        "Image": "media:fixture",
                        "State": "running",
                        "Status": "Up (healthy)",
                        "Labels": {
                            "com.docker.compose.project": "media",
                            "secret": "never-export",
                        },
                    }
                ]
            elif "/stats" in self.path:
                value = {
                    "cpu_stats": {
                        "cpu_usage": {"total_usage": int(time.monotonic() * 100000)},
                        "system_cpu_usage": int(time.monotonic() * 1000000),
                        "online_cpus": 2,
                    },
                    "memory_stats": {
                        "usage": 60,
                        "limit": 100,
                        "stats": {"inactive_file": 10},
                    },
                }
            elif "/events" in self.path:
                self.send_response(200)
                self.send_header("Content-Type", "application/json")
                self.end_headers()
                try:
                    for _ in range(5):
                        self.wfile.write(
                            b'{"Type":"container","Action":"health_status"}\n'
                        )
                        self.wfile.flush()
                        time.sleep(1)
                except (BrokenPipeError, ConnectionResetError):
                    pass
                return
            else:
                self.send_error(404)
                return
            data = json.dumps(value).encode()
            self.send_response(200)
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)

    class Server(ThreadingHTTPServer, UnixStreamServer):
        address_family = socket.AF_UNIX

    docker = Path("/var/run/docker.sock")
    if docker.exists():
        raise RuntimeError("unexpected Docker socket in fixture VM")
    server = Server(str(docker), Handler)
    os.chmod(docker, 0o660)
    os.chown(docker, 0, int(run("getent", "group", "docker").stdout.split(":")[2]))
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    run("systemctl", "restart", "limeos-shadow-containerd")
    deadline = time.monotonic() + 35
    while time.monotonic() < deadline:
        view = http_request(8004, "/api/v1/overview", cookie=shadow_cookie)[2]
        if any(r["kind"] == "container" for r in view["resources"]):
            break
        time.sleep(0.5)
    else:
        raise RuntimeError("container reconciliation failed")
    assert "never-export" not in json.dumps(view)
    assert any(
        r["id"] == f"container:{container_id}" and r["status"] == "running"
        for r in view["resources"]
    )
    inventory_calls = calls["/v1.41/containers/json"]
    durations = []
    for _ in range(90):
        start = time.monotonic()
        assert http_request(8004, "/api/v1/overview", cookie=shadow_cookie)[0] == 200
        durations.append((time.monotonic() - start) * 1000)
    assert calls["/v1.41/containers/json"] <= inventory_calls + 1
    passed("GET-only bounded Engine reconciliation shared by repeated dashboard reads")
    # Stream interruption and reconnect returns a complete authorized snapshot.
    for _ in range(2):
        connection = http.client.HTTPConnection("127.0.0.1", 8004, timeout=5)
        connection.request(
            "GET",
            "/api/v1/observations/stream",
            headers={"Cookie": shadow_cookie, "Last-Event-ID": "1"},
        )
        response = connection.getresponse()
        assert response.status == 200
        first = response.readline()
        assert first.startswith(b"event: snapshot")
        connection.close()
    for path in ["/api/v1/containers/restart", "/api/v1/resources/format"]:
        assert http_request(8004, path, cookie=shadow_cookie)[0] == 404
    passed("SSE reconnect/interrupted readers and absent mutation routes")
    time.sleep(1)
    memory = {
        u: pss(int(run("systemctl", "show", u, "-p", "MainPID", "--value").stdout))
        for u in shadow_units
    }
    assert sum(memory.values()) < 30 * 1024
    server.shutdown()
    server.server_close()
    docker.unlink()
    run("apt-get", "remove", "-y", "limeos-shadow")
    assert original == hashes(reference) and original_state == authority_snapshot()
    assert mounts == Path("/proc/1/mountinfo").read_text()
    for u in units:
        run("systemctl", "is-active", u)
    assert http_request(8003, "/api/v1/auth/session", cookie=baseline_cookie)[0] == 200
    passed(
        "shadow removal leaves existing login, services, mounts and reference state unchanged"
    )
    output.write_text(
        json.dumps(
            {
                "result": "pass",
                "checks": results,
                "shadow_pss_kib": memory,
                "combined_pss_kib": sum(memory.values()),
                "cached_http_p95_ms": sorted(durations)[int(len(durations) * 0.95)],
                "engine_calls": dict(calls),
                "scope": "Disposable x86-64 Debian VM with synthetic Engine; not reference Pi hardware.",
            },
            indent=2,
        )
        + "\n"
    )


if __name__ == "__main__":
    main()
