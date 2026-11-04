"""Real observation timer/reader and in-app delivery, disposable Debian guest only."""

import hashlib
import json
import os
import signal
import sqlite3
import subprocess
import sys
import tarfile
import time
from contextlib import closing
from pathlib import Path

import p03_guest as api
import p04_mounts_guest as engine_fixture
from current_authority_schema import current_authority_schema

VERSION = "0.4.12+gateabort.1"
DB = Path("/var/lib/limeos/core/core.sqlite")
BASE = "/api/v1/automation"
run = engine_fixture.run
require = engine_fixture.require

# Fixed guest-only observer. It has no source, socket, UID or clock override.
HEALTH_PROBE = r"""import json, os, pwd, socket, struct, sys, time
started = time.monotonic()
deadline = started + 4
nonce = sys.argv[1]
if len(nonce) != 64 or any(c not in '0123456789abcdef' for c in nonce):
    raise ValueError('invalid observation nonce')
uid = os.geteuid()
if uid != pwd.getpwnam('limeos-core').pw_uid:
    raise PermissionError('real Core UID required')
def remaining():
    value = deadline - time.monotonic()
    if value <= 0:
        raise TimeoutError('original probe deadline expired')
    return value
with socket.socket(socket.AF_UNIX) as stream:
    stream.settimeout(remaining())
    stream.connect('/run/limeos-storage-reader/executor.sock')
    pid, peer_uid, gid = struct.unpack('3i', stream.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, 12))
    if pid <= 1 or peer_uid != 0:
        raise PermissionError('real root reader peer required')
    body = json.dumps({'operation': 'container_health', 'version': 1, 'nonce': nonce}).encode()
    stream.settimeout(remaining())
    stream.sendall(struct.pack('!I', len(body)) + body)
    def receive(size):
        result = bytearray()
        while len(result) < size:
            stream.settimeout(remaining())
            part = stream.recv(size - len(result))
            if not part:
                raise EOFError('incomplete reader frame')
            result.extend(part)
        remaining()
        return result
    size = struct.unpack('!I', receive(4))[0]
    if not 0 < size <= 65536:
        raise ValueError('reader frame exceeds bound')
    reply = json.loads(receive(size))
    remaining()
    print(json.dumps({'reply': reply, 'client_uid': uid, 'peer': {'pid': pid, 'uid': peer_uid, 'gid': gid}, 'nonce': nonce, 'elapsed_seconds': time.monotonic() - started}))
"""

OBSERVATION_ERRORS = (
    OSError,
    ValueError,
    TypeError,
    KeyError,
    RecursionError,
    RuntimeError,
    sqlite3.Error,
    subprocess.SubprocessError,
)


def health_probe():
    """One later independent read; never a timer response or settlement input."""
    nonce = os.urandom(32).hex()
    value = run(
        "runuser",
        "-u",
        "limeos-core",
        "-g",
        "limeos-host-access",
        "--",
        "python3",
        "-c",
        HEALTH_PROBE,
        nonce,
        check=False,
        timeout=5,
    )
    require(len(value.stdout.encode()) <= 64 * 1024, "probe stdout exceeds bound")
    if value.returncode:
        return {
            "returncode": value.returncode,
            "stderr": value.stderr[-2048:],
            "nonce": nonce,
        }
    observed = json.loads(value.stdout)
    require(observed["nonce"] == nonce, "observation nonce changed")
    return {"returncode": 0, **observed}


def read_bound(path, limit):
    with Path(path).open("rb") as stream:
        data = stream.read(limit + 1)
    require(len(data) <= limit, "observation file exceeds bound")
    return data.decode()


def service_snapshot(name):
    require(
        name in ("limeos-core", "limeos-storage-reader"), "unexpected observed unit"
    )
    result = {"unit": unit(name)}
    pid = int(result["unit"]["MainPID"])
    require(pid > 1, "observed unit has no live process")
    proc = Path("/proc") / str(pid)
    raw = read_bound(proc / "stat", 4096)
    result["pid"] = pid
    result["start_ticks"] = int(raw[raw.rindex(")") + 1 :].split()[19])
    selected = (
        "Uid",
        "Gid",
        "CapEff",
        "CapPrm",
        "CapBnd",
        "NoNewPrivs",
        "Threads",
        "VmRSS",
        "VmHWM",
    )
    result["status"] = {
        key: value.strip()
        for line in read_bound(proc / "status", 8192).splitlines()
        if ":" in line
        for key, value in [line.split(":", 1)]
        if key in selected
    }
    with os.scandir(proc / "fd") as entries:
        count = 0
        for _ in entries:
            count += 1
            require(count <= 4096, "FD count exceeds observation bound")
    result["fd_count"] = count
    expected = "/system.slice/" + name + ".service"
    require(result["unit"]["ControlGroup"] == expected, "unexpected observation cgroup")
    result["cgroup"] = {}
    for field in (
        "memory.current",
        "memory.peak",
        "memory.events",
        "pids.current",
        "pids.events",
    ):
        try:
            result["cgroup"][field] = read_bound(
                Path("/sys/fs/cgroup" + expected) / field, 4096
            )
        except OBSERVATION_ERRORS as error:
            result["cgroup"][field] = {
                "available": False,
                "error_type": type(error).__name__,
            }
    return result


def failure_snapshot():
    disposable_guard()
    result = {
        "scope": "post-failure independent read-only observations, not the timer reply, an atomic snapshot or authority",
        "observed_at": time.time(),
    }

    def capture(name, observe, limit):
        try:
            value = observe()
            require(
                len(json.dumps(value).encode()) <= limit,
                "observation section exceeds bound",
            )
            result[name] = {"available": True, "value": value}
        except OBSERVATION_ERRORS as error:
            result[name] = {"available": False, "error_type": type(error).__name__}

    capture("runtime", lambda: request("/schedules"), 16 * 1024)
    capture("health", health_probe, 40 * 1024)
    capture("core", lambda: service_snapshot("limeos-core"), 12 * 1024)
    capture("reader", lambda: service_snapshot("limeos-storage-reader"), 12 * 1024)
    require(
        len(json.dumps(result).encode()) <= 96 * 1024,
        "observation packet exceeds bound",
    )
    return result


def record_failure_observation(evidence):
    try:
        value = failure_snapshot()
    except OBSERVATION_ERRORS as error:
        value = {"available": False, "error_type": type(error).__name__}
    evidence["boundary_observations"] = value
    print("B1_OBSERVATION " + json.dumps(value, sort_keys=True), flush=True)


def disposable_guard():
    if (
        os.geteuid() != 0
        or Path("/etc/hostname").read_text().strip() != "limeos-p01-test"
    ):
        raise SystemExit("Refusing outside the disposable LimeOS VM")


def sha(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def failure_packet(evidence):
    encoded = json.dumps(evidence, sort_keys=True)
    if len(encoded.encode()) <= 128 * 1024:
        return encoded
    value = {
        key: evidence[key]
        for key in ("summary", "stage", "package_version", "fixture_sha256")
        if key in evidence
    }
    value["error"] = str(evidence.get("error", ""))[:1024]
    value["details"] = (
        "unavailable in bounded stdout; result-file persistence is not assumed"
    )
    return json.dumps(value, sort_keys=True)


def history(path=DB):
    """One bounded query-only snapshot; connection and statements close on all paths."""
    tables = {
        "schedules": "SELECT id,revision,paused,settled_through FROM automation_schedules ORDER BY id",
        "occurrences": "SELECT schedule,revision,due,job FROM automation_occurrences ORDER BY schedule,revision,due",
        "jobs": "SELECT id,state,digest FROM jobs WHERE json_extract(intent,'$.operation')='automation_observe_health' ORDER BY id",
        "results": "SELECT job,outcome,reason,sequence,complete FROM automation_observation_results ORDER BY job",
        "alerts": "SELECT key,body FROM automation_alerts ORDER BY owner,key",
        "notifications": "SELECT id,job,body FROM automation_notifications ORDER BY id",
    }
    with closing(
        sqlite3.connect(Path(path).resolve().as_uri() + "?mode=ro", uri=True, timeout=2)
    ) as conn:
        conn.execute("PRAGMA query_only=ON").close()
        conn.execute("BEGIN").close()
        result = {}
        for name, statement in tables.items():
            with closing(conn.execute(statement + " LIMIT 129")) as cursor:
                rows = cursor.fetchall()
            require(len(rows) <= 128, "fixture history row bound exceeded")
            result[name] = [list(row) for row in rows]
        conn.rollback()
    require(
        len(json.dumps(result).encode()) <= 96 * 1024,
        "fixture history byte bound exceeded",
    )
    return result


def notification(snapshot, key, event, generation=None):
    rows = [json.loads(row[2]) for row in snapshot["notifications"]]
    matching = [
        row
        for row in rows
        if row["event"]["key"] == key and row["event"]["event"] == event
    ]
    if not matching:
        return None
    require(len(matching) == 1, "duplicate notification transition")
    value = matching[0]
    if generation is not None:
        require(
            value["event"]["generation"] == generation, "recovery generation changed"
        )
    return value if value["delivery"]["state"] == "delivered" else None


def assert_identities(snapshot):
    tuples = [tuple(row[:3]) for row in snapshot["occurrences"]]
    require(len(tuples) == len(set(tuples)), "duplicate occurrence tuple")
    jobs = [row[0] for row in snapshot["jobs"]]
    results = [row[0] for row in snapshot["results"]]
    require(
        len(jobs) == len(set(jobs)) and len(results) == len(set(results)),
        "duplicate job/result",
    )
    require(set(jobs) == set(results), "observation job has no exact result")
    require(
        all(row[1] == "succeeded" for row in snapshot["jobs"]),
        "observation job did not succeed",
    )
    require(
        all(
            row[1:3] == ["succeeded", "observed"] and row[4] == 0
            for row in snapshot["results"]
        ),
        "wrong or falsely complete observation result",
    )
    require(
        {row[3] for row in snapshot["occurrences"]} == set(jobs),
        "occurrence/job binding differs",
    )
    for _, job, body in snapshot["notifications"]:
        require(
            job in jobs and json.loads(body)["channel"] == "in_app",
            "notification binding/channel differs",
        )


def unit(name):
    properties = run(
        "systemctl",
        "show",
        name,
        "-p",
        "MainPID",
        "-p",
        "ControlGroup",
        "-p",
        "MemoryMax",
        "-p",
        "TasksMax",
        "-p",
        "LimitNOFILE",
        "-p",
        "KillMode",
    ).stdout
    return dict(line.split("=", 1) for line in properties.splitlines() if "=" in line)


def restart_core():
    before = int(unit("limeos-core")["MainPID"])
    require(before > 1, "Core is not live")
    os.kill(before, signal.SIGKILL)

    def restarted():
        value = unit("limeos-core")
        if int(value["MainPID"]) <= 1 or int(value["MainPID"]) == before:
            return None
        if api.http("/api/v1/auth/session", cookie=credentials[0])[0] != 200:
            return None
        return {"before_pid": before, "after": value}

    return api.eventually(restarted, timeout=20)


credentials = (None, None)


def request(
    path,
    method="GET",
    body=None,
    *,
    expected=200,
    csrf=True,
    origin="https://localhost",
    authenticated=True,
):
    code, _, value = api.http(
        BASE + path,
        method,
        body,
        credentials[0] if authenticated else None,
        credentials[1] if csrf else None,
        origin,
    )
    require(
        code == expected,
        f"unexpected B1 API status {code}, expected {expected}: {value!r}",
    )
    return value


def wait_notification(key, event, generation=None, timeout=75):
    def observed():
        # Require the authenticated API, not only the root observer's SQLite view.
        page = request("/notifications")
        require(page["partial"] is True, "container-only coverage became complete")
        value = notification(history(), key, event, generation)
        return value if value and any(row == value for row in page["items"]) else None

    return api.eventually(observed, timeout=timeout)


def main(package_version=VERSION):
    global credentials
    disposable_guard()
    repo, output = map(Path, sys.argv[1:])
    evidence = {
        "stage": "install",
        "package_version": package_version,
        "passed": [],
        "limitations": [
            "AMD64 installed API/runtime fixture; no browser, ARM64/Pi, upgrade/return or soak acceptance",
            "Container-only partial observation; no mount, deployment or external notification effect",
        ],
    }

    def passed(message):
        evidence["passed"].append(message)
        print("PASS " + message, flush=True)

    try:
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
        run("apt-get", "update", timeout=120)
        run(
            "apt-get",
            "install",
            "-y",
            "limeos=" + package_version,
            "busybox-static",
            timeout=180,
        )
        api.eventually(lambda: Path("/run/limeos-core/core.sock").exists())
        evidence["authority_schema"] = current_authority_schema()
        with closing(sqlite3.connect(DB.as_uri() + "?mode=ro", uri=True)) as conn:
            require(
                conn.execute("PRAGMA user_version").fetchone()[0]
                == evidence["authority_schema"],
                "installed schema differs from trusted source",
            )
        evidence["binary_sha256"] = {
            name: sha("/usr/lib/limeos/" + name)
            for name in (
                "limeos-core",
                "limeos-executor",
                "limeos-stack-read-worker",
                "limeos-password-worker",
                "limeosctl",
            )
        }
        evidence["package_sha256"] = {
            p.name: sha(p) for p in sorted(repo.rglob("*.deb"))
        }
        evidence["fixture_sha256"] = sha(__file__)
        passed(
            "signed current package installation and independently sourced authority schema"
        )
        evidence["stage"] = "real_engine"
        evidence["docker"] = engine_fixture.actual_docker()
        rootfs = Path("/root/b1-rootfs.tar")
        with tarfile.open(rootfs, "w") as archive:
            archive.add("/bin/busybox", arcname="bin/busybox")
        run("docker", "import", str(rootfs), "limeos-b1-fixture:local")
        identifier = run(
            "docker",
            "create",
            "--name",
            "limeos-b1-health",
            "--restart",
            "unless-stopped",
            "--network",
            "none",
            "limeos-b1-fixture:local",
            "/bin/busybox",
            "sleep",
            "3600",
        ).stdout.strip()
        require(
            len(identifier) == 64 and all(c in "0123456789abcdef" for c in identifier),
            "invalid real container ID",
        )
        key = "container:" + identifier
        evidence["container_id"] = identifier
        evidence["container_before"] = json.loads(
            run("docker", "inspect", identifier).stdout
        )[0]["State"]
        require(
            not evidence["container_before"]["Running"], "fixture must begin stopped"
        )
        evidence["stage"] = "authentication"
        cookie, csrf, _principal = api.enroll()
        credentials = (cookie, csrf)
        before = history()
        schedule_input = {
            "interval_seconds": 30,
            "grace_seconds": 29,
            "missed": "coalesce_latest",
            "window": None,
        }
        request("/schedules", "POST", schedule_input, expected=403)
        require(history() == before, "missing management grant changed history")
        response = api.http(
            "/api/v1/auth/capabilities/observe-health/enable",
            "POST",
            {"password": "Synthetic-P03-test-password"},
            cookie,
            csrf,
        )
        require(
            response[0] == 200 and response[2]["enabled"],
            "actual health enablement failed",
        )
        cookie, csrf, _principal = api.enroll()
        credentials = (cookie, csrf)
        before = history()
        for changes in (
            {"csrf": False},
            {"origin": "https://invalid.example"},
            {"authenticated": False},
        ):
            request(
                "/schedules",
                "POST",
                schedule_input,
                expected=401 if "authenticated" in changes else 403,
                **changes,
            )
            require(history() == before, "refused schedule mutation changed history")
        policy = request("/health-policy")
        policy["definition"]["fail_threshold"] = 1
        evidence["policy"] = request(
            "/health-policy",
            "POST",
            {"revision": policy["revision"], "definition": policy["definition"]},
        )
        passed(
            "actual password enablement, relogin and permission/CSRF/Origin refusals preserve history"
        )
        evidence["stage"] = "protected_reader"
        policy_path = Path("/etc/limeos/system-policy/storage.json")
        evidence["reader_policy_sha256"] = sha(policy_path)
        evidence["reader_policy"] = json.loads(policy_path.read_text())
        require(
            evidence["reader_policy"]["allow_container_health_read"],
            "package health ceiling is disabled",
        )
        require(
            run(
                "systemctl", "is-active", "limeos-storage-reader", check=False
            ).returncode
            != 0,
            "reader was not dormant",
        )
        run("systemctl", "start", "limeos-storage-reader")
        from guest import executor_probe_code
        from p04_fixture_readiness import wait_storage_reader

        evidence["reader_ready"] = wait_storage_reader(run, executor_probe_code())
        evidence["reader_unit"] = unit("limeos-storage-reader")
        pid = int(evidence["reader_unit"]["MainPID"])
        status = Path(f"/proc/{pid}/status").read_text()
        require(
            "CapEff:\t0000000000000000" in status and "NoNewPrivs:\t1" in status,
            "reader gained capabilities",
        )
        require(
            evidence["reader_unit"]["MemoryMax"] == "67108864"
            and evidence["reader_unit"]["TasksMax"] == "16"
            and evidence["reader_unit"]["LimitNOFILE"] == "256",
            "reader limits changed",
        )
        evidence["stage"] = "incident"
        schedule = request("/schedules", "POST", schedule_input, expected=201)
        evidence["created_schedule"] = schedule
        require(schedule["status"] == "scheduled", "ordinary producer not registered")
        opening = wait_notification(key, "incident")
        evidence["opening"] = opening
        original = history()
        assert_identities(original)
        require(
            len(original["jobs"]) == 1, "first observation did not settle exactly once"
        )
        evidence["before_restart"] = original
        evidence["restarts"] = [restart_core()]
        stable_until = time.monotonic() + 8
        while time.monotonic() < stable_until:
            require(
                history() == original,
                "restart/stable observations changed original history",
            )
            time.sleep(0.25)
        evidence["after_restart"] = history()
        passed(
            "real root reader and Core timer publish one exact incident; actual Core SIGKILL/restart preserves every observation identity"
        )
        evidence["stage"] = "recovery"
        run("docker", "start", identifier)
        state = json.loads(run("docker", "inspect", identifier).stdout)[0]["State"]
        require(state["Running"] is True, "independent Engine running check failed")
        evidence["container_after"] = state
        recovery = wait_notification(key, "recovery", opening["event"]["generation"])
        evidence["recovery"] = recovery
        current = history()
        assert_identities(current)
        require(len(current["jobs"]) == 2, "recovery did not settle exactly once")
        alerts = request("/alerts")
        require(
            alerts["partial"] is True
            and len(alerts["items"]) == 1
            and alerts["items"][0]["incident"]["phase"] == "clear",
            "recovery alert not clear/partial",
        )
        evidence["recovered_alerts"] = alerts
        # Recoveries are delivered only after the original incident was visible.
        acknowledged = request(
            "/notifications/" + opening["id"] + "/acknowledgment",
            "POST",
            {"revision": opening["revision"]},
        )
        require(
            acknowledged["acknowledged_at"] is not None,
            "notification acknowledgment missing",
        )
        paused = request(
            "/schedules/" + schedule["schedule"]["id"] + "/pause",
            "POST",
            {"revision": schedule["schedule"]["revision"], "paused": True},
        )
        require(paused["status"] == "paused", "schedule not paused")
        request(
            "/schedules/" + schedule["schedule"]["id"] + "/pause",
            "POST",
            {"revision": schedule["schedule"]["revision"], "paused": False},
            expected=409,
        )
        before_restart = history()
        evidence["acknowledged"] = acknowledged
        evidence["paused"] = paused
        evidence["restarts"].append(restart_core())
        time.sleep(2)
        require(
            history() == before_restart,
            "paused/acknowledged history changed across restart",
        )
        page = request("/schedules")
        require(page["items"] == [paused], "paused schedule did not survive restart")
        require(
            sha(policy_path) == evidence["reader_policy_sha256"],
            "fixture changed installed reader policy",
        )
        evidence["final_history"] = history()
        passed(
            "same-container recovery is delivered once; acknowledgment, stale-revision refusal and pause survive another real Core restart"
        )
        evidence.update(
            stage="complete",
            summary="pass",
            architecture=run("dpkg", "--print-architecture").stdout.strip(),
        )
    except Exception as error:
        evidence.update(summary="fail", error=f"{type(error).__name__}: {error}")
        record_failure_observation(evidence)
        try:
            evidence["history_after_failure"] = history()
        except (OSError, sqlite3.Error, ValueError) as observation_error:
            evidence["history_after_failure"] = {
                "unavailable": type(observation_error).__name__
            }
        evidence["unit_diagnostics"] = run(
            "journalctl",
            "-u",
            "limeos-storage-reader",
            "-u",
            "limeos-core",
            "--no-pager",
            "-n",
            "40",
            "-o",
            "cat",
            check=False,
            timeout=5,
        ).stdout[-24000:]
        print("B1_FAILURE " + failure_packet(evidence), flush=True)
        raise
    finally:
        encoded = json.dumps(evidence, indent=2) + "\n"
        require(
            len(encoded.encode()) <= 2 * 1024 * 1024, "fixture result exceeds bound"
        )
        output.write_text(encoded)


if __name__ == "__main__":
    main()
