#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Destructive P01 acceptance tests. Run only inside a disposable systemd VM."""

import http.client
import json
import os
import pwd
import socket
import struct
import subprocess
import sys
import tempfile
import time
from datetime import datetime, timezone
from pathlib import Path


def run(*args, check=True, input=None, env=None):
    result = subprocess.run(
        args, check=False, input=input, text=True, capture_output=True, env=env
    )
    if check and result.returncode != 0:
        raise RuntimeError(
            f"{args[0]} failed ({result.returncode}): {result.stderr[-4096:]}"
        )
    return result


def http_request(method, path, body=None, headers=None):
    connection = http.client.HTTPConnection("127.0.0.1", 8003, timeout=12)
    supplied = {"Origin": "https://localhost", "Content-Type": "application/json"}
    supplied.update(headers or {})
    connection.request(
        method,
        path,
        body=json.dumps(body) if body is not None else None,
        headers=supplied,
    )
    response = connection.getresponse()
    data = response.read()
    result = (
        response.status,
        dict(response.getheaders()),
        json.loads(data) if data else None,
    )
    connection.close()
    return result


def rpc(request):
    data = json.dumps(request).encode()
    with socket.socket(socket.AF_UNIX) as stream:
        stream.settimeout(6)
        stream.connect("/run/limeos-core/core.sock")
        stream.sendall(struct.pack("!I", len(data)) + data)
        size = struct.unpack("!I", stream.recv(4))[0]
        body = b""
        while len(body) < size:
            body += stream.recv(size - len(body))
        return json.loads(body)


def refused_executor_reply(body, error):
    # The current protocol sends a bounded refusal. EOF/reset is also a refusal;
    # a successful health response or any observation/effect evidence is not.
    if body.strip() == "denied":
        return True
    try:
        value = json.loads(body)
    except json.JSONDecodeError:
        return False
    return value == {"version": 1, "ready": False, "error": error}


def main(package_version="0.1.0", upgrade_version="0.1.1"):
    if (
        os.geteuid() != 0
        or Path("/etc/hostname").read_text().strip() != "limeos-p01-test"
    ):
        raise SystemExit("Refusing tests outside the disposable LimeOS VM.")
    repository = Path(sys.argv[1])
    output = Path(sys.argv[2])
    results = []

    def passed(name):
        results.append(name)
        print("PASS " + name, flush=True)

    run(
        "install",
        "-m",
        "0644",
        str(repository / "limeos-archive-keyring.gpg"),
        "/usr/share/keyrings/limeos-test.gpg",
    )
    Path("/etc/apt/sources.list.d/limeos-test.list").write_text(
        "deb [signed-by=/usr/share/keyrings/limeos-test.gpg] file:"
        + str(repository)
        + " stable main\n"
    )
    run("apt-get", "update")
    run("apt-get", "install", "-y", "limeos=" + package_version)
    run("/usr/lib/limeos/limeosctl", "status")
    units = ["limeos-core", "limeos-containerd", "limeos-storaged"]
    for unit in units:
        run("systemctl", "is-active", unit)
    passed("signed apt install and systemd activation")
    original_ids = [
        pwd.getpwnam(name).pw_uid
        for name in ["limeos-core", "limeos-containerd", "limeos-assistant"]
    ]
    assert all(100 <= uid < 1000 for uid in original_ids)
    for path in Path("/usr/lib/limeos").rglob("*"):
        assert not path.is_symlink()
        meta = path.stat()
        assert meta.st_uid == 0 and meta.st_mode & 0o022 == 0
    for unit in ["limeos-core", "limeos-containerd"]:
        assert run(
            "systemctl", "show", unit, "-p", "User", "--value"
        ).stdout.strip() not in ["", "root"]
    for account in ["limeos-core", "limeos-containerd", "limeos-assistant"]:
        assert pwd.getpwnam(account).pw_shell == "/usr/sbin/nologin"
    executable = Path("/usr/lib/limeos/limeos-executor")
    libraries = []
    for line in run("ldd", str(executable)).stdout.splitlines():
        libraries.extend(Path(token) for token in line.split() if token.startswith("/"))
    for file in [executable, *libraries]:
        for component in [file, *file.parents]:
            metadata = component.stat()
            assert metadata.st_uid == 0 and metadata.st_mode & 0o022 == 0, str(
                component
            )
    root_pid = int(
        run("systemctl", "show", "limeos-storaged", "-p", "MainPID", "--value").stdout
    )
    assert Path(f"/proc/{root_pid}/exe").resolve() == executable
    passed("root-owned payload and dedicated non-login accounts")
    assert (
        http_request(
            "POST", "/api/v1/auth/login", {"username": "admin", "password": "pihealth"}
        )[0]
        == 401
    )
    issued = json.loads(run("/usr/lib/limeos/limeosctl", "bootstrap").stdout)
    password = "disposable-fixture-password"
    enrollment = {"token": issued["token"], "username": "alice", "password": password}
    run("/usr/lib/limeos/limeosctl", "enroll", input=json.dumps(enrollment))
    assert (
        run(
            "/usr/lib/limeos/limeosctl",
            "enroll",
            input=json.dumps(enrollment),
            check=False,
        ).returncode
        != 0
    )
    assert run("/usr/lib/limeos/limeosctl", "bootstrap", check=False).returncode != 0
    assert (
        run(
            "runuser",
            "-u",
            "nobody",
            "--",
            "/usr/lib/limeos/limeosctl",
            "bootstrap",
            check=False,
        ).returncode
        != 0
    )
    passed("no default login and one-use local root enrollment")
    status, headers, session = http_request(
        "POST", "/api/v1/auth/login", {"username": "alice", "password": password}
    )
    assert status == 200
    cookie = headers["set-cookie"]
    assert all(
        part in cookie for part in ["Secure", "HttpOnly", "SameSite=Strict", "Path=/"]
    )
    token_cookie = cookie.split(";")[0]
    assert (
        http_request("POST", "/api/v1/auth/logout", headers={"Cookie": token_cookie})[0]
        == 403
    )
    assert (
        http_request(
            "POST",
            "/api/v1/auth/logout",
            headers={"Cookie": token_cookie, "X-CSRF-Token": "f" * 64},
        )[0]
        == 403
    )
    assert (
        http_request(
            "POST",
            "/api/v1/auth/logout",
            headers={
                "Origin": "https://evil.invalid",
                "Cookie": token_cookie,
                "X-CSRF-Token": session["csrf_token"],
            },
        )[0]
        == 403
    )
    assert http_request("GET", "/api/v1/auth/logout")[0] == 405
    assert (
        http_request(
            "POST",
            "/api/v1/auth/logout",
            headers={"Cookie": token_cookie, "X-CSRF-Token": session["csrf_token"]},
        )[0]
        == 204
    )
    assert (
        http_request("GET", "/api/v1/auth/session", headers={"Cookie": token_cookie})[0]
        == 401
    )
    passed("secure cookies, origin, CSRF, GET safety and revocation")
    assert (
        rpc({"request": "health", "version": 1, "username": "alice"})["response"]
        == "error"
    )
    assert (
        rpc(
            {
                "request": "check_task",
                "version": 1,
                "token": "f" * 64,
                "task": "forged",
                "operation": "storage_manage",
                "resource": "boot",
            }
        )["response"]
        == "error"
    )
    with socket.socket(socket.AF_UNIX) as stream:
        stream.settimeout(6)
        stream.connect("/run/limeos-core/core.sock")
        stream.sendall(struct.pack("!I", 65537))
        assert stream.recv(4)
    for account in ["nobody", "limeos-core", "limeos-assistant"]:
        for endpoint in [
            "/run/limeos-storaged/executor.sock",
            "/run/limeos-containerd/executor.sock",
        ]:
            code = (
                "import socket; s=socket.socket(socket.AF_UNIX); s.connect('"
                + endpoint
                + "')"
            )
            assert (
                run(
                    "runuser", "-u", account, "--", "python3", "-c", code, check=False
                ).returncode
                != 0
            )
    # Group membership only admits the connection: the daemon checks kernel UID.
    probe = r"""import socket, struct, json, sys
s=socket.socket(socket.AF_UNIX); s.settimeout(6); s.connect(sys.argv[1])
request=json.loads(sys.argv[2]); body=json.dumps(request).encode()
try:
    s.sendall(struct.pack('!I',len(body))+body)
except (BrokenPipeError, ConnectionResetError):
    # Kernel UID rejection may close before our first write. Read any buffered
    # refusal instead of discarding it; callers still validate the exact reply.
    pass
try:
    header=s.recv(4)
except (BrokenPipeError, ConnectionResetError):
    header=b''
if not header:
    print('denied'); sys.exit(0)
size=struct.unpack('!I',header)[0]; body=b''
while len(body)<size:
    chunk=s.recv(size-len(body))
    if not chunk: raise RuntimeError('truncated response')
    body+=chunk
print(body.decode())
"""
    for executor, group in [
        ("limeos-storaged", "limeos-host-access"),
        ("limeos-containerd", "limeos-container-access"),
    ]:
        endpoint = f"/run/{executor}/executor.sock"
        args = ["--", "python3", "-c", probe, endpoint]
        result = run(
            "runuser",
            "-u",
            "limeos-core",
            "-g",
            group,
            *args,
            json.dumps({"operation": "health", "version": 1}),
        )
        assert json.loads(result.stdout)["ready"] is True
        denied = run(
            "runuser",
            "-u",
            "nobody",
            "-g",
            group,
            *args,
            json.dumps({"operation": "health", "version": 1}),
        )
        assert refused_executor_reply(denied.stdout, "forbidden")
        forged = run(
            "runuser",
            "-u",
            "limeos-core",
            "-g",
            group,
            *args,
            json.dumps({"operation": "health", "version": 1, "actor": "root"}),
        )
        assert refused_executor_reply(forged.stdout, "invalid_input")
        # A dead daemon must not qualify as successful refusal.
        healthy = run(
            "runuser",
            "-u",
            "limeos-core",
            "-g",
            group,
            *args,
            json.dumps({"operation": "health", "version": 1}),
        )
        assert json.loads(healthy.stdout)["ready"] is True
    assert (
        run(
            "runuser",
            "-u",
            "nobody",
            "--",
            "test",
            "-w",
            "/usr/lib/limeos/limeos-executor",
            check=False,
        ).returncode
        != 0
    )
    # Simulate a Docker endpoint with Docker's actual mode/group and prove separation.
    with socket.socket(socket.AF_UNIX) as docker:
        docker.bind("/run/limeos-test-docker.sock")
        docker.listen(4)
        os.chown(
            "/run/limeos-test-docker.sock",
            0,
            __import__("grp").getgrnam("docker").gr_gid,
        )
        os.chmod("/run/limeos-test-docker.sock", 0o660)
        for account in ["limeos-core", "limeos-assistant"]:
            assert (
                run(
                    "runuser",
                    "-u",
                    account,
                    "--",
                    "python3",
                    "-c",
                    "import socket; s=socket.socket(socket.AF_UNIX); s.connect('/run/limeos-test-docker.sock')",
                    check=False,
                ).returncode
                != 0
            )
        run(
            "runuser",
            "-u",
            "limeos-containerd",
            "-g",
            "docker",
            "--",
            "python3",
            "-c",
            "import socket; s=socket.socket(socket.AF_UNIX); s.connect('/run/limeos-test-docker.sock')",
        )
    passed("forged authority, oversized RPC and socket access denied")
    with socket.create_connection(("127.0.0.1", 8003), timeout=7) as slow:
        slow.sendall(b"G")
        start = time.monotonic()
        assert slow.recv(1024) == b""
        assert time.monotonic() - start < 6.5
    with socket.create_connection(("127.0.0.1", 8003), timeout=7) as oversized:
        oversized.sendall(
            b"GET /api/v1/health HTTP/1.1\r\nHost: localhost\r\nX-Large: "
            + b"x" * 20000
            + b"\r\n\r\n"
        )
        response = oversized.recv(1024)
        assert not response.startswith(b"HTTP/1.1 200")
    passed("slow and oversized HTTP headers bounded")
    # Reinstall from a sudo account and verify the account identities stay fixed.
    run("apt-get", "install", "-y", "sudo")
    run("adduser", "--disabled-password", "--gecos", "", "test-installer")
    Path("/etc/sudoers.d/limeos-test").write_text(
        "test-installer ALL=(root) NOPASSWD: /usr/bin/apt-get\n"
    )
    Path("/etc/sudoers.d/limeos-test").chmod(0o440)
    run(
        "runuser",
        "-u",
        "test-installer",
        "--",
        "sudo",
        "apt-get",
        "install",
        "-y",
        "--reinstall",
        "limeos=" + package_version,
    )
    assert original_ids == [
        pwd.getpwnam(name).pw_uid
        for name in ["limeos-core", "limeos-containerd", "limeos-assistant"]
    ]
    # Existing system UID alone must not allow a login account or Docker access.
    run("usermod", "--shell", "/bin/bash", "limeos-core")
    assert (
        run("/var/lib/dpkg/info/limeos.postinst", "configure", check=False).returncode
        != 0
    )
    run("usermod", "--shell", "/usr/sbin/nologin", "limeos-core")
    run("adduser", "limeos-core", "docker")
    assert (
        run("/var/lib/dpkg/info/limeos.postinst", "configure", check=False).returncode
        != 0
    )
    run("deluser", "limeos-core", "docker")
    run("/var/lib/dpkg/info/limeos.postinst", "configure")
    config = Path("/etc/limeos/core.json")
    valid = config.read_bytes()
    config.write_bytes(b"{broken")
    assert (
        run("/var/lib/dpkg/info/limeos.postinst", "configure", check=False).returncode
        != 0
    )
    assert config.read_bytes() == b"{broken"
    config.write_bytes(valid)
    run("/var/lib/dpkg/info/limeos.postinst", "configure")
    policy = Path("/etc/limeos/system-policy/host.json")
    old_policy = policy.read_bytes()
    policy.write_bytes(b"{corrupt")
    assert (
        run("/var/lib/dpkg/info/limeos.postinst", "configure", check=False).returncode
        != 0
    )
    assert policy.read_bytes() == b"{corrupt"
    policy.write_bytes(old_policy)
    run("/var/lib/dpkg/info/limeos.postinst", "configure")
    restrictive = json.loads(old_policy)
    restrictive["allow_health"] = False
    policy.write_text(json.dumps(restrictive))
    run("/var/lib/dpkg/info/limeos.postinst", "configure")
    assert json.loads(policy.read_text())["allow_health"] is False
    policy.write_bytes(old_policy)
    run("/var/lib/dpkg/info/limeos.postinst", "configure")
    passed("root/sudo rerun identity and corrupt configuration preserved")
    with tempfile.TemporaryDirectory() as temp:
        fake = Path(temp) / "systemctl"
        env = {**os.environ, "PATH": temp + ":" + os.environ["PATH"]}
        for action in ["daemon-reload", "enable", "restart"]:
            fake.write_text(
                '#!/bin/sh\n[ "$1" = "'
                + action
                + '" ] && exit 23\nexec /usr/bin/systemctl "$@"\n'
            )
            fake.chmod(0o755)
            assert (
                run(
                    "/var/lib/dpkg/info/limeos.postinst",
                    "configure",
                    env=env,
                    check=False,
                ).returncode
                != 0
            )
            assert config.read_bytes() == valid and policy.read_bytes() == old_policy
        fake.write_text(
            '#!/bin/sh\n[ "$1" = "stop" ] && exit 23\nexec /usr/bin/systemctl "$@"\n'
        )
        fake.chmod(0o755)
        assert (
            run(
                "/var/lib/dpkg/info/limeos.prerm", "remove", env=env, check=False
            ).returncode
            != 0
        )
    run("/var/lib/dpkg/info/limeos.postinst", "configure")
    passed("activation and removal failures surface and rerun recovers")
    run("apt-get", "install", "-y", "limeos=" + upgrade_version)
    run("apt-get", "install", "-y", "--allow-downgrades", "limeos=" + package_version)
    run("/usr/lib/limeos/limeosctl", "status")
    assert config.read_bytes() == valid
    assert (
        http_request(
            "POST", "/api/v1/auth/login", {"username": "alice", "password": password}
        )[0]
        == 200
    )
    assert not run("dpkg", "--verify", "limeos").stdout.strip()
    payload = Path("/usr/lib/limeos/ui/index.html")
    original_payload = payload.read_bytes()
    try:
        payload.write_bytes(original_payload + b"tampered")
        assert "index.html" in run("dpkg", "--verify", "limeos").stdout
    finally:
        payload.write_bytes(original_payload)
    assert not run("dpkg", "--verify", "limeos").stdout.strip()
    passed("upgrade/downgrade preserve identity/configuration and package payload")
    # Actual low-space pressure must reject writes while keeping reads available.
    pressure = Path("/var/lib/limeos/core/.test-pressure")
    available = (
        os.statvfs(pressure.parent).f_bavail * os.statvfs(pressure.parent).f_frsize
    )
    try:
        run("fallocate", "-l", str(available - 4 * 1024 * 1024), str(pressure))
        assert (
            http_request(
                "POST",
                "/api/v1/auth/login",
                {"username": "alice", "password": password},
            )[0]
            == 503
        )
        assert http_request("GET", "/api/v1/health")[0] == 200
    finally:
        pressure.unlink(missing_ok=True)
    passed("actual space pressure refuses state changes and preserves reads")
    startup_ms = []
    status_ms = []
    for _ in range(5):
        # Earlier install/failure cases intentionally restart repeatedly. Reset
        # the counter for each benchmark, preserving the production burst limit.
        run("systemctl", "reset-failed", "limeos-core")
        started = time.monotonic()
        run("systemctl", "restart", "limeos-core")
        deadline = started + 2
        while time.monotonic() < deadline:
            if run("/usr/lib/limeos/limeosctl", "status", check=False).returncode == 0:
                break
            time.sleep(0.01)
        else:
            raise RuntimeError("Core did not become ready within two seconds")
        startup_ms.append(round((time.monotonic() - started) * 1000, 3))
    assert max(startup_ms) < 1000, f"Startup exceeded one second: {startup_ms}"
    for _ in range(10):
        started = time.monotonic()
        run("/usr/lib/limeos/limeosctl", "status")
        status_ms.append(round((time.monotonic() - started) * 1000, 3))
    # Measure after authentication rather than a newly started, untouched core.
    assert (
        http_request(
            "POST", "/api/v1/auth/login", {"username": "alice", "password": password}
        )[0]
        == 200
    )
    passed("five core restarts reach readiness below one second")
    measurements = {}
    for unit in units:
        pid = int(run("systemctl", "show", unit, "-p", "MainPID", "--value").stdout)
        fields = {
            line.split(":")[0]: int(line.split()[1])
            for line in Path(f"/proc/{pid}/smaps_rollup").read_text().splitlines()[1:]
            if ":" in line
        }
        measurements[unit] = {"pss_kib": fields["Pss"], "rss_kib": fields["Rss"]}
        cgroup = run(
            "systemctl", "show", unit, "-p", "ControlGroup", "--value"
        ).stdout.strip()
        for name in ["memory.peak", "memory.swap.current"]:
            metric = Path("/sys/fs/cgroup" + cgroup) / name
            if metric.exists():
                measurements[unit][name.replace(".", "_") + "_bytes"] = int(
                    metric.read_text()
                )
    combined_pss = sum(m["pss_kib"] for m in measurements.values())
    assert combined_pss <= 30 * 1024, (
        f"Idle resident services exceed 30 MiB: {combined_pss} KiB"
    )
    assert not run(
        "pgrep", "-f", "^/usr/lib/limeos/limeos-password-worker$", check=False
    ).stdout
    passed("post-authentication idle footprint stays below 30 MiB")
    before = Path("/var/lib/limeos/core/core.sqlite-wal").stat().st_mtime_ns
    for _ in range(50):
        assert http_request("GET", "/api/v1/health")[0] == 200
    assert before == Path("/var/lib/limeos/core/core.sqlite-wal").stat().st_mtime_ns
    logs = run("journalctl", "-u", "limeos-core", "--no-pager").stdout
    assert (
        password not in logs
        and issued["token"] not in logs
        and session["csrf_token"] not in logs
    )
    passed("idle health reads do not write and logs contain no credentials")
    run("apt-get", "remove", "-y", "limeos")
    assert Path("/var/lib/limeos/core/core.sqlite").exists()
    run("apt-get", "install", "-y", "limeos=" + package_version)
    assert (
        http_request(
            "POST", "/api/v1/auth/login", {"username": "alice", "password": password}
        )[0]
        == 200
    )
    passed("remove/reinstall preserves authority and existing login")
    output.write_text(
        json.dumps(
            {
                "platform": "Debian 12 x86-64 KVM",
                "date_utc": datetime.now(timezone.utc).isoformat(),
                "kernel": run("uname", "-r").stdout.strip(),
                "vcpus": 2,
                "ram_mib": 1024,
                "tests": results,
                "measurements": measurements,
                "startup_ms": startup_ms,
                "status_cli_ms": status_ms,
                "combined_pss_kib": combined_pss,
                "result": "pass",
            },
            indent=2,
        )
        + "\n"
    )


if __name__ == "__main__":
    main()
