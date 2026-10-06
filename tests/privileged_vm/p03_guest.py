#!/usr/bin/env python3
"""Real Engine restart qualification; exclusively inside the disposable Debian VM."""

import hashlib
import json
import os
import pwd
import signal
import socket
import sqlite3
import statistics
import subprocess
import sys
import tarfile
import threading
import time
from http import client as http_client
from http.server import BaseHTTPRequestHandler
from pathlib import Path
from socketserver import ThreadingMixIn, UnixStreamServer

CORE_DB = "/var/lib/limeos/core/core.sqlite"
RECEIPTS = "/var/lib/limeos/executors/container/receipts.sqlite"
REAL_SOCKET = "/var/run/limeos-p03-real.sock"
BASE = "/api/v1/container/restart"
PACKAGE_VERSION = "0.3.0"
checks = []


def run(*args, input=None, check=True):
    result = subprocess.run(
        args, input=input, text=True, capture_output=True, check=False
    )
    if check and result.returncode:
        raise RuntimeError(f"{args[0]} failed: {result.stderr[-2000:]}")
    return result


def passed(name):
    checks.append(name)
    print("PASS " + name, flush=True)


def http(
    path,
    method="GET",
    body=None,
    cookie=None,
    csrf=None,
    origin="https://localhost",
    port=8003,
):
    connection = http_client.HTTPConnection("127.0.0.1", port, timeout=12)
    headers = {"Origin": origin, "Content-Type": "application/json"}
    if cookie:
        headers["Cookie"] = cookie
    if csrf:
        headers["X-CSRF-Token"] = csrf
    connection.request(
        method, path, json.dumps(body) if body is not None else None, headers
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


def eventually(test, timeout=45):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            value = test()
            if value:
                return value
        except (OSError, AssertionError, sqlite3.Error):
            pass
        time.sleep(0.1)
    raise AssertionError("Timed out waiting for disposable-host state")


def docker(*args):
    return run("docker", "--host", "unix://" + REAL_SOCKET, *args).stdout.strip()


def inspect(identifier):
    return json.loads(docker("inspect", identifier))[0]


def restart_container():
    # Policy tests deliberately reload faster than an operator would. Reset
    # only this setup counter; crash cases use automatic recovery unchanged.
    run("systemctl", "reset-failed", "limeos-containerd")
    run("systemctl", "restart", "limeos-containerd")
    wait_container_ready()


def wait_container_ready():
    # Type=exec guarantees execve, not that policy/receipt initialization and
    # the Unix listener are ready. Probe as the authorized kernel UID.
    code = "import json,socket,struct; s=socket.socket(socket.AF_UNIX); s.settimeout(2); s.connect('/run/limeos-containerd/executor.sock'); d=json.dumps({'operation':'health','version':1}).encode(); s.sendall(struct.pack('!I',len(d))+d); n=struct.unpack('!I',s.recv(4))[0]; b=b''\nwhile len(b)<n: b+=s.recv(n-len(b))\nprint(b.decode())"

    last_error = ""

    def ready():
        nonlocal last_error
        result = run(
            "runuser",
            "-u",
            "limeos-core",
            "-g",
            "limeos-container-access",
            "--",
            "python3",
            "-c",
            code,
            check=False,
        )
        last_error = result.stderr[-1000:]
        return result.returncode == 0 and json.loads(result.stdout).get("ready") is True

    try:
        eventually(ready, timeout=10)
    except AssertionError as error:
        raise AssertionError(
            "Executor readiness probe failed: " + last_error
        ) from error


def enroll(prefix="limeos", port=8003):
    ctl = [f"/usr/lib/{prefix}/limeosctl", "--socket", f"/run/{prefix}-core/core.sock"]
    bootstrap = run(*ctl, "bootstrap", check=False)
    if bootstrap.returncode == 0:
        token = json.loads(bootstrap.stdout)["token"]
        run(
            *ctl,
            "enroll",
            input=json.dumps(
                {
                    "token": token,
                    "username": "test-admin",
                    "password": "Synthetic-P03-test-password",
                }
            ),
        )
    status, headers, view = http(
        "/api/v1/auth/login",
        "POST",
        {"username": "test-admin", "password": "Synthetic-P03-test-password"},
        port=port,
        origin="https://localhost"
        if port == 8003
        else "https://limeos-shadow.localhost:8444",
    )
    assert status == 200
    return headers["set-cookie"].split(";")[0], view["csrf_token"], view["principal"]


class UnixConnection(http_client.HTTPConnection):
    def connect(self):
        self.sock = socket.socket(socket.AF_UNIX)
        self.sock.settimeout(40)
        self.sock.connect(REAL_SOCKET)


class ProxyControl:
    def __init__(self):
        self.mode = ""
        self.identifier = ""
        self.entered = threading.Event()
        self.release = threading.Event()
        self.posts = {}
        self.effects_ms = {}
        self.lock = threading.Lock()

    def arm(self, mode, identifier):
        self.mode, self.identifier = mode, identifier
        self.entered.clear()
        self.release.clear()

    def barrier(self):
        self.entered.set()
        if not self.release.wait(20):
            raise RuntimeError("Fault barrier timed out")


control = ProxyControl()


class Proxy(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *_):
        pass

    def do_GET(self):
        self.forward()

    def do_POST(self):
        identifier = self.path.split("/")[3] if "/containers/" in self.path else ""
        with control.lock:
            control.posts[identifier] = control.posts.get(identifier, 0) + 1
        if control.mode == "during" and identifier == control.identifier:
            control.barrier()
        self.forward()

    def forward(self):
        if (
            control.mode == "before_prepare"
            and control.identifier in self.path
            and self.path.endswith("/json")
        ):
            with sqlite3.connect(CORE_DB) as db:
                running = db.execute(
                    "SELECT EXISTS(SELECT 1 FROM jobs WHERE resource=? AND state='running')",
                    ("container:" + control.identifier,),
                ).fetchone()[0]
            if running:
                control.barrier()
        length = int(self.headers.get("Content-Length", "0"))
        connection = UnixConnection("localhost", timeout=40)
        started = time.monotonic()
        connection.request(
            self.command, self.path, self.rfile.read(length) if length else None
        )
        response = connection.getresponse()
        data = response.read()
        connection.close()
        if self.command == "POST":
            identifier = self.path.split("/")[3]
            with control.lock:
                control.effects_ms[identifier] = round(
                    (time.monotonic() - started) * 1000, 3
                )
        if (
            control.mode == "after_effect"
            and self.command == "POST"
            and control.identifier in self.path
        ):
            control.barrier()
        try:
            self.send_response(response.status)
            self.send_header("Content-Length", str(len(data)))
            self.send_header(
                "Content-Type", response.getheader("Content-Type", "application/json")
            )
            self.end_headers()
            self.wfile.write(data)
        except (BrokenPipeError, ConnectionResetError):
            pass


class Server(ThreadingMixIn, UnixStreamServer):
    daemon_threads = True

    def handle_error(self, *_):
        pass


def plan(identifier, cookie, csrf):
    status, _, proposal = http(
        BASE + "/plans", "POST", {"resource": "container:" + identifier}, cookie, csrf
    )
    assert status == 201, (status, proposal)
    status, _, approval = http(
        BASE + f"/plans/{proposal['plan']['id']}/approval",
        "POST",
        {"digest": proposal["digest"]},
        cookie,
        csrf,
    )
    assert status == 200
    return proposal, approval["token"]


def queue(proposal, approval, key, cookie, csrf):
    body = {"proposal": proposal, "approval": approval, "idempotency_key": key}
    status, _, job = http(BASE + "/jobs", "POST", body, cookie, csrf)
    assert status == 202, (status, job)
    return job, body


def state(identifier):
    with sqlite3.connect(CORE_DB) as db:
        return db.execute(
            "SELECT state FROM jobs WHERE id=?", (identifier,)
        ).fetchone()[0]


def rpc_as(account, request):
    code = "import json,socket,struct,sys; d=sys.stdin.buffer.read(); s=socket.socket(socket.AF_UNIX); s.settimeout(8); s.connect('/run/limeos-core/core.sock'); s.sendall(struct.pack('!I',len(d))+d); n=struct.unpack('!I',s.recv(4))[0]; b=b''\nwhile len(b)<n: b+=s.recv(n-len(b))\nprint(b.decode())"
    result = run(
        "runuser",
        "-u",
        account,
        "-g",
        "limeos-rpc",
        "--",
        "python3",
        "-c",
        code,
        input=json.dumps(request),
    )
    return json.loads(result.stdout)


def main():
    if os.getuid() != 0 or socket.gethostname() != "limeos-p01-test":
        raise SystemExit("Disposable guest required")
    repository, output = map(Path, sys.argv[1:])
    if not Path("/usr/bin/docker").exists() or not Path("/bin/busybox").exists():
        run("apt-get", "update", "-qq")
        run(
            "apt-get",
            "install",
            "--no-install-recommends",
            "-y",
            "docker.io",
            "busybox-static",
        )
    eventually(lambda: Path("/var/run/docker.sock").exists())
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
    run("apt-get", "update", "-qq")
    run("apt-get", "install", "-y", "limeos=" + PACKAGE_VERSION)
    cookie, csrf, principal = enroll()
    # A real Docker daemon, an image constructed from Debian's static busybox,
    # and private writable container layers. No bind mounts or production data.
    os.rename("/var/run/docker.sock", REAL_SOCKET)
    rootfs = Path("/root/p03-rootfs.tar")
    with tarfile.open(rootfs, "w") as archive:
        archive.add("/bin/busybox", arcname="bin/busybox")
    docker("import", str(rootfs), "limeos-p03-fixture:local")
    ids = [
        docker(
            "run",
            "-d",
            "--name",
            f"p03-{n}",
            "--label",
            "limeos.fixture=redacted",
            "limeos-p03-fixture:local",
            "/bin/busybox",
            "sleep",
            "3600",
        )
        for n in range(10)
    ]
    server = Server("/var/run/docker.sock", Proxy)
    os.chown(
        "/var/run/docker.sock", 0, pwd.getpwnam("limeos-containerd").pw_gid
    )  # reset to docker group below
    import grp

    os.chown("/var/run/docker.sock", 0, grp.getgrnam("docker").gr_gid)
    os.chmod("/var/run/docker.sock", 0o660)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    policy_path = Path("/etc/limeos/system-policy/container.json")
    policy = json.loads(policy_path.read_text())
    assert not policy["allow_restart"] and policy["managed_containers"] == []
    policy.update(allow_restart=True, managed_containers=ids[:9])
    policy_path.write_text(json.dumps(policy))
    restart_container()
    for account in ["limeos-core", "limeos-assistant"]:
        result = run(
            "runuser",
            "-u",
            account,
            "--",
            "python3",
            "-c",
            "import socket; s=socket.socket(socket.AF_UNIX); s.connect('/var/run/docker.sock')",
            check=False,
        )
        assert result.returncode != 0
        assert "docker" not in run("id", "-nG", account).stdout.split()
        assert (
            run(
                "runuser", "-u", account, "--", "test", "-r", RECEIPTS, check=False
            ).returncode
            != 0
        )
    passed("only the container executor can access Engine and protected receipts")

    before = inspect(ids[0])["State"]["StartedAt"]
    proposal, approval = plan(ids[0], cookie, csrf)
    for path in [
        "/plans",
        f"/plans/{proposal['plan']['id']}/approval",
        f"/plans/{proposal['plan']['id']}/cancel",
        f"/jobs/{'a' * 64}/cancel",
    ]:
        assert http(BASE + path, cookie=cookie)[0] == 405
    assert (
        http(
            BASE + "/plans",
            "POST",
            {"resource": "container:" + ids[0]},
            cookie,
            "a" * 64,
        )[0]
        == 403
    )
    assert (
        http(
            BASE + "/plans",
            "POST",
            {"resource": "container:" + ids[0]},
            cookie,
            csrf,
            origin="https://foreign.invalid",
        )[0]
        == 403
    )
    assert control.posts.get(ids[0], 0) == 0
    passed("GET and invalid origin or CSRF cannot initiate an operation")
    job, body = queue(proposal, approval, "normal", cookie, csrf)
    eventually(lambda: state(job["id"]) == "succeeded")
    assert inspect(ids[0])["State"]["StartedAt"] != before
    assert control.posts[ids[0]] == 1
    assert http(BASE + "/jobs", "POST", body, cookie, csrf)[2]["id"] == job["id"]
    assert control.posts[ids[0]] == 1
    progress = http(BASE + f"/jobs/{job['id']}", cookie=cookie)[2]
    assert progress["events"][-1]["event"]["state"] == "succeeded"
    with sqlite3.connect(CORE_DB) as db:
        results_table = (
            "container_results"
            if db.execute("PRAGMA user_version").fetchone()[0] >= 4
            else "restart_results"
        )
        receipt, verification = db.execute(
            f"SELECT receipt,verification FROM {results_table} WHERE job=?",
            (job["id"],),
        ).fetchone()
        assert (
            json.loads(receipt)["state"] == "verified"
            and json.loads(verification)["running"]
        )
    passed(
        "one real restart, independent verification, durable progress and duplicate request"
    )

    proposal, approval = plan(ids[1], cookie, csrf)
    altered = json.loads(json.dumps(proposal))
    altered["plan"]["expected"]["resource"] = "container:" + ids[2]
    assert (
        http(
            BASE + "/jobs",
            "POST",
            {"proposal": altered, "approval": approval, "idempotency_key": "altered"},
            cookie,
            csrf,
        )[0]
        == 409
    )
    docker("restart", "-t", "1", ids[1])
    assert (
        http(
            BASE + "/jobs",
            "POST",
            {"proposal": proposal, "approval": approval, "idempotency_key": "stale"},
            cookie,
            csrf,
        )[0]
        == 409
    )
    proposal, approval = plan(ids[9], cookie, csrf)
    forbidden, _ = queue(proposal, approval, "ceiling", cookie, csrf)
    eventually(lambda: state(forbidden["id"]) == "needs_intervention")
    assert control.posts.get(ids[9], 0) == 0
    passed("changed plans, stale incarnation and independent managed-ID ceiling")

    # Inject a scoped P01 identity/token fixture; no principal or role is accepted
    # on either live API. Task protocol intentionally has no approval operation.
    token, session = "e" * 64, "f" * 64
    digest = lambda value: hashlib.sha256(value.encode()).hexdigest()
    scoped_id = "d" * 64
    scoped_csrf = digest("csrf:" + session)
    with sqlite3.connect(CORE_DB) as db:
        hashed = db.execute(
            "SELECT password_hash FROM users WHERE id=?", (principal["id"],)
        ).fetchone()[0]
        db.execute(
            "INSERT INTO users VALUES(?,?,?,?,1)",
            (scoped_id, "scoped", hashed, '"operator"'),
        )
        db.execute(
            "INSERT INTO grants VALUES(?,?,?)",
            (scoped_id, '"container_manage"', "container:" + ids[2]),
        )
        db.execute(
            "INSERT INTO sessions VALUES(?,?,?,?,?,0)",
            (
                digest(session),
                scoped_id,
                digest(scoped_csrf),
                1,
                int(time.time()) + 3600,
            ),
        )
        generation = db.execute("SELECT generation FROM meta").fetchone()[0]
        scopes = json.dumps(
            [{"operation": "container_manage", "resource": "container:" + ids[2]}]
        )
        db.execute(
            "INSERT INTO task_tokens VALUES(?,?,?,?,?,?,?, ?,0)",
            (
                digest(token),
                scoped_id,
                pwd.getpwnam("limeos-assistant").pw_uid,
                "test-task",
                scopes,
                1,
                generation,
                int(time.time()) + 3600,
            ),
        )
    scoped_cookie = "__Host-limeos=" + session
    request = {
        "request": "propose_restart",
        "version": 1,
        "token": token,
        "task": "test-task",
        "resource": "container:" + ids[2],
    }
    assistant = rpc_as("limeos-assistant", request)
    assert assistant["response"] == "restart_plan", assistant
    assert (
        http(
            BASE + "/plans",
            "POST",
            {"resource": request["resource"]},
            scoped_cookie,
            scoped_csrf,
        )[0]
        == 201
    )
    request["resource"] = "container:" + ids[3]
    assert rpc_as("limeos-assistant", request)["code"] == "forbidden"
    assert (
        http(
            BASE + "/plans",
            "POST",
            {"resource": request["resource"]},
            scoped_cookie,
            scoped_csrf,
        )[0]
        == 403
    )
    assert (
        rpc_as("limeos-assistant", {**request, "request": "approve_restart"})["code"]
        == "invalid_input"
    )
    # Internally tagged newtype responses flatten the plan wrapper.
    assistant_plan = {"plan": assistant["plan"], "digest": assistant["digest"]}
    status, _, human_approval = http(
        BASE + f"/plans/{assistant_plan['plan']['id']}/approval",
        "POST",
        {"digest": assistant_plan["digest"]},
        scoped_cookie,
        scoped_csrf,
    )
    assert status == 200
    accepted = rpc_as(
        "limeos-assistant",
        {
            "request": "queue_restart",
            "version": 1,
            "token": token,
            "task": "test-task",
            "input": {
                "proposal": assistant_plan,
                "approval": human_approval["token"],
                "idempotency_key": "task-approved",
            },
        },
    )
    assert accepted["response"] == "restart_job", accepted
    eventually(lambda: state(accepted["id"]) == "succeeded")
    with sqlite3.connect(CORE_DB) as db:
        db.execute("UPDATE users SET grant_revision=2 WHERE id=?", (scoped_id,))
    request["resource"] = "container:" + ids[2]
    assert rpc_as("limeos-assistant", request)["code"] == "expired"
    assert (
        http(
            BASE + "/plans",
            "POST",
            {"resource": request["resource"]},
            scoped_cookie,
            scoped_csrf,
        )[0]
        == 401
    )
    passed(
        "browser and scoped task share policy; task cannot approve; grant revocation is immediate"
    )

    # Advance only this throwaway VM's wall clock; the test never waits five
    # minutes or changes the workstation clock to prove approval expiry.
    proposal, approval = plan(ids[1], cookie, csrf)
    original_time = int(time.time())
    run("systemctl", "stop", "systemd-timesyncd", check=False)
    try:
        run("date", "--set", "@" + str(original_time + 301))
        status, _, error = http(
            BASE + "/jobs",
            "POST",
            {"proposal": proposal, "approval": approval, "idempotency_key": "expired"},
            cookie,
            csrf,
        )
        assert status == 403 and error["code"] == "expired"
    finally:
        run("date", "--set", "@" + str(original_time))
    assert control.posts.get(ids[1], 0) == 0
    passed("expired human approval causes no Engine effect")

    # Occupy the single bounded dispatcher with one allowed operation. The
    # second queued job can then be canceled without relying on a timing race.
    blocker_plan, blocker_approval = plan(ids[0], cookie, csrf)
    control.arm("during", ids[0])
    blocker, _ = queue(blocker_plan, blocker_approval, "cancel-barrier", cookie, csrf)
    assert control.entered.wait(12)
    cancel_plan, cancel_approval = plan(ids[1], cookie, csrf)
    canceled, canceled_input = queue(
        cancel_plan, cancel_approval, "cancel-queued", cookie, csrf
    )
    assert (
        http(BASE + f"/jobs/{canceled['id']}/cancel", "POST", cookie=cookie, csrf=csrf)[
            0
        ]
        == 204
    )
    assert (
        http(BASE + f"/jobs/{blocker['id']}/cancel", "POST", cookie=cookie, csrf=csrf)[
            0
        ]
        == 409
    )
    control.release.set()
    control.mode = ""
    eventually(lambda: state(blocker["id"]) == "succeeded")
    assert state(canceled["id"]) == "canceled" and control.posts.get(ids[1], 0) == 0
    assert (
        http(BASE + "/jobs", "POST", canceled_input, cookie, csrf)[2]["state"]
        == "canceled"
    )
    passed(
        "queued cancellation prevents effects; in-flight cancellation cannot claim prevention"
    )

    for index, (mode, target) in enumerate(
        [
            (m, t)
            for m in ["before_prepare", "during", "after_effect"]
            for t in ["limeos-core", "limeos-containerd"]
        ],
        start=3,
    ):
        identifier = ids[index]
        old = inspect(identifier)["State"]["StartedAt"]
        proposal, approval = plan(identifier, cookie, csrf)
        control.arm(mode, identifier)
        job, _ = queue(proposal, approval, f"kill-{mode}-{target}", cookie, csrf)
        # Busybox is PID 1 in this fixture: Engine may use the entire fixed
        # 10-second stop grace before returning its response. The after-effect
        # barrier also follows the dispatch tick and selected inspections.
        assert control.entered.wait(25), (mode, target, state(job["id"]))
        pid = int(run("systemctl", "show", target, "-p", "MainPID", "--value").stdout)
        os.kill(pid, signal.SIGKILL)
        control.release.set()
        # Stop arming before a restarted core begins independent reconciliation.
        control.mode = ""
        eventually(
            lambda target=target: (
                run("systemctl", "is-active", target, check=False).stdout.strip()
                == "active"
            )
        )
        expected = "succeeded" if target == "limeos-core" else "needs_intervention"
        eventually(lambda job=job, expected=expected: state(job["id"]) == expected)
        if target == "limeos-containerd" and mode == "before_prepare":
            assert (
                control.posts.get(identifier, 0) == 0
                and inspect(identifier)["State"]["StartedAt"] == old
            )
        else:
            assert control.posts.get(identifier, 0) == 1
        time.sleep(0.5)
        assert control.posts.get(identifier, 0) <= 1
        if target == "limeos-containerd" and mode != "before_prepare":
            with sqlite3.connect(RECEIPTS) as db:
                assert (
                    db.execute(
                        "SELECT state FROM actions WHERE action=?", (job["id"],)
                    ).fetchone()[0]
                    == "prepared"
                )
        passed(f"kill {target} {mode}: no unjustified second effect")

    # Shadow uses a distinct receipt directory and refuses a writable ceiling.
    run("apt-get", "install", "-y", "limeos-shadow=" + PACKAGE_VERSION)
    shadow_cookie, shadow_csrf, _ = enroll("limeos-shadow", 8004)
    assert (
        http(
            BASE + "/plans",
            "POST",
            {"resource": "container:" + ids[0]},
            shadow_cookie,
            shadow_csrf,
            origin="https://limeos-shadow.localhost:8444",
            port=8004,
        )[0]
        == 201
    )
    shadow_policy = Path("/etc/limeos-shadow/system-policy/container.json")
    value = json.loads(shadow_policy.read_text())
    value.update(allow_restart=True, managed_containers=[ids[0]])
    shadow_policy.write_text(json.dumps(value))
    run("systemctl", "restart", "limeos-shadow-containerd", check=False)
    eventually(
        lambda: (
            run(
                "systemctl",
                "show",
                "limeos-shadow-containerd",
                "-p",
                "Result",
                "--value",
            ).stdout.strip()
            == "exit-code"
        ),
        timeout=10,
    )
    assert (
        "limeos-shadow/executors/container"
        in run("systemctl", "cat", "limeos-shadow-containerd").stdout
    )
    passed("shadow has isolated state and refuses write-enabled policy")
    standard_pid = run(
        "systemctl", "show", "limeos-core", "-p", "MainPID", "--value"
    ).stdout
    run("apt-get", "remove", "-y", "limeos-shadow")
    assert (
        run("systemctl", "show", "limeos-core", "-p", "MainPID", "--value").stdout
        == standard_pid
    )
    for unit in ["limeos-core", "limeos-containerd", "limeos-storaged"]:
        assert run("systemctl", "is-active", unit).stdout.strip() == "active"
    assert http("/api/v1/overview", cookie=cookie)[0] == 200
    passed("shadow removal preserves standard services and authenticated reads")
    memory = {}
    for unit in ["limeos-core", "limeos-containerd", "limeos-storaged"]:
        pid = int(run("systemctl", "show", unit, "-p", "MainPID", "--value").stdout)
        fields = {
            line.split(":")[0]: line.split(":")[1].strip()
            for line in Path(f"/proc/{pid}/smaps_rollup").read_text().splitlines()
            if ":" in line
        }
        memory[unit] = int(fields["Pss"].split()[0])
        assert int(fields["Swap"].split()[0]) == 0
    elapsed = []
    for _ in range(20):
        started = time.monotonic()
        assert http("/api/v1/overview", cookie=cookie)[0] == 200
        elapsed.append((time.monotonic() - started) * 1000)
    output.write_text(
        json.dumps(
            {
                "checks": checks,
                "engine_version": docker("version", "--format", "{{.Server.Version}}"),
                "container_fixture": "Debian static busybox, no mounts, redacted label",
                "restart_posts": sum(control.posts.values()),
                "restart_effect_ms": control.effects_ms,
                "footprint": {
                    "architecture": run("dpkg", "--print-architecture").stdout.strip(),
                    "app_pss_kib": memory,
                    "combined_pss_kib": sum(memory.values()),
                    "overview_p95_ms": round(sorted(elapsed)[18], 3),
                    "overview_median_ms": round(statistics.median(elapsed), 3),
                    "workload": "after real restart/interruption qualification, 20 cached overview requests",
                    "exclusions": [
                        "Docker daemon",
                        "test proxy",
                        "browser",
                        "Rust build tools",
                    ],
                    "swap_kib": 0,
                },
                "package_sha256": {
                    p.name: hashlib.sha256(p.read_bytes()).hexdigest()
                    for p in sorted(
                        repository.glob(f"pool/main/*{PACKAGE_VERSION}*.deb")
                    )
                },
            },
            indent=2,
        )
        + "\n"
    )
    return cookie, csrf, principal, ids, policy_path, policy


if __name__ == "__main__":
    main()
