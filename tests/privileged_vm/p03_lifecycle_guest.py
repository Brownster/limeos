#!/usr/bin/env python3
"""Lifecycle, logs and real schema upgrade qualification in the disposable guest."""

import hashlib
import json
import os
import signal
import socket
import sqlite3
import statistics
import sys
import tarfile
import time
from pathlib import Path

sys.path.insert(0, "/root/build/source/tests/privileged_vm")
import p03_guest as fixture

BASE = "/api/v1/container"


def repository(path):
    fixture.run(
        "install",
        "-m",
        "0644",
        str(path / "limeos-archive-keyring.gpg"),
        "/usr/share/keyrings/limeos-test.gpg",
    )
    Path("/etc/apt/sources.list.d/limeos-test.list").write_text(
        f"deb [signed-by=/usr/share/keyrings/limeos-test.gpg] file:{path} stable main\n"
    )
    fixture.run("apt-get", "update", "-qq")


def upgrade(candidate, package_version="0.3.2", authority_schema=5):
    previous = Path("/opt/limeos-previous-repo")
    assert previous.is_dir(), "The runner must retain the qualified 0.3.0 repository"
    repository(previous)
    fixture.run("apt-get", "install", "-y", "limeos=0.3.0")
    cookie, csrf, _ = fixture.enroll()
    rootfs = Path("/root/upgrade-rootfs.tar")
    with tarfile.open(rootfs, "w") as archive:
        archive.add("/bin/busybox", arcname="bin/busybox")
    fixture.run("docker", "import", str(rootfs), "limeos-upgrade:local")
    identifier = fixture.run(
        "docker", "run", "-d", "limeos-upgrade:local", "/bin/busybox", "sleep", "3600"
    ).stdout.strip()
    policy_path = Path("/etc/limeos/system-policy/container.json")
    policy = json.loads(policy_path.read_text())
    policy.update(allow_restart=True, managed_containers=[identifier])
    policy_path.write_text(json.dumps(policy))
    fixture.restart_container()
    proposal, approval = fixture.plan(identifier, cookie, csrf)
    old_job, old_body = fixture.queue(
        proposal, approval, "upgrade-verified", cookie, csrf
    )
    fixture.eventually(lambda: fixture.state(old_job["id"]) == "succeeded")
    pending, token = fixture.plan(identifier, cookie, csrf)
    with sqlite3.connect(fixture.CORE_DB) as db:
        assert db.execute("PRAGMA user_version").fetchone()[0] == 3
    with sqlite3.connect(fixture.RECEIPTS) as db:
        assert db.execute("PRAGMA user_version").fetchone()[0] == 2
    repository(candidate)
    fixture.run("apt-get", "install", "-y", "limeos=" + package_version)
    fixture.eventually(
        lambda: fixture.http("/api/v1/overview", cookie=cookie)[0] == 200
    )
    assert (
        fixture.http(fixture.BASE + "/jobs", "POST", old_body, cookie, csrf)[2]["id"]
        == old_job["id"]
    )
    new_job, _ = fixture.queue(pending, token, "upgrade-pending", cookie, csrf)
    fixture.eventually(lambda: fixture.state(new_job["id"]) == "succeeded")
    with sqlite3.connect(fixture.CORE_DB) as db:
        assert db.execute("PRAGMA user_version").fetchone()[0] == authority_schema
        assert db.execute("SELECT COUNT(*) FROM container_results").fetchone()[0] == 2
    with sqlite3.connect(fixture.RECEIPTS) as db:
        assert db.execute("PRAGMA user_version").fetchone()[0] == 3
        assert (
            db.execute(
                "SELECT COUNT(*) FROM actions WHERE operation='restart' AND state='verified'"
            ).fetchone()[0]
            == 2
        )
    policy.update(allow_restart=False, managed_containers=[])
    policy_path.write_text(json.dumps(policy))
    fixture.restart_container()
    fixture.run("docker", "rm", "-f", identifier)
    fixture.passed(
        f"real 0.3.0 to {package_version} upgrade preserves sessions, approved restart bytes and verified receipts"
    )


def plan(identifier, action, cookie, csrf):
    status, _, proposal = fixture.http(
        BASE + "/plans",
        "POST",
        {"resource": "container:" + identifier, "operation": action},
        cookie,
        csrf,
    )
    assert status == 201, (action, status, proposal)
    assert proposal["plan"].get("operation", "restart") == action
    status, _, approval = fixture.http(
        BASE + f"/plans/{proposal['plan']['id']}/approval",
        "POST",
        {"digest": proposal["digest"]},
        cookie,
        csrf,
    )
    assert status == 200, (status, approval)
    return proposal, approval["token"]


def queue(proposal, approval, key, cookie, csrf):
    body = {"proposal": proposal, "approval": approval, "idempotency_key": key}
    status, _, job = fixture.http(BASE + "/jobs", "POST", body, cookie, csrf)
    assert status == 202, (status, job)
    return job, body


def main(package_version="0.3.2", authority_schema=5, qualify_upgrade=True):
    if os.getuid() != 0 or socket.gethostname() != "limeos-p01-test":
        raise SystemExit("Disposable guest required")
    candidate, output = map(Path, sys.argv[1:])
    if not Path("/usr/bin/docker").exists() or not Path("/bin/busybox").exists():
        fixture.run("apt-get", "update", "-qq")
        fixture.run(
            "apt-get",
            "install",
            "--no-install-recommends",
            "-y",
            "docker.io",
            "busybox-static",
        )
    fixture.eventually(
        lambda: fixture.run("docker", "info", check=False).returncode == 0
    )
    if qualify_upgrade and Path("/opt/limeos-previous-repo").is_dir():
        upgrade(candidate, package_version, authority_schema)
    fixture.PACKAGE_VERSION = package_version
    cookie, csrf, principal, ids, policy_path, policy = fixture.main()
    extra = [
        fixture.docker(
            "run",
            "-d",
            "--name",
            f"lifecycle-{n}",
            "limeos-p03-fixture:local",
            "/bin/busybox",
            "sleep",
            "3600",
        )
        for n in range(14)
    ]
    policy.update(
        allow_start=True,
        allow_stop=True,
        allow_container_logs=True,
        managed_containers=ids[:9] + extra,
    )
    policy_path.write_text(json.dumps(policy))
    fixture.restart_container()
    identifier = extra[0]
    for action in ["stop", "start"]:
        before = fixture.inspect(identifier)["State"]
        proposal, approval = plan(identifier, action, cookie, csrf)
        altered = json.loads(json.dumps(proposal))
        altered["plan"]["operation"] = "restart"
        assert (
            fixture.http(
                BASE + "/jobs",
                "POST",
                {
                    "proposal": altered,
                    "approval": approval,
                    "idempotency_key": "changed-" + action,
                },
                cookie,
                csrf,
            )[0]
            == 409
        )
        assert (
            fixture.http(
                fixture.BASE + "/jobs",
                "POST",
                {
                    "proposal": proposal,
                    "approval": approval,
                    "idempotency_key": "legacy-" + action,
                },
                cookie,
                csrf,
            )[0]
            == 400
        )
        job, body = queue(proposal, approval, "normal-" + action, cookie, csrf)
        fixture.eventually(lambda job=job: fixture.state(job["id"]) == "succeeded")
        after = fixture.inspect(identifier)["State"]
        assert after["Running"] == (action == "start")
        assert (after["StartedAt"] != before["StartedAt"]) == (action == "start")
        count = fixture.control.posts[identifier]
        assert (
            fixture.http(BASE + "/jobs", "POST", body, cookie, csrf)[2]["id"]
            == job["id"]
        )
        assert fixture.control.posts[identifier] == count
        assert (
            fixture.http(
                BASE + "/plans",
                "POST",
                {"resource": "container:" + identifier, "operation": action},
                cookie,
                csrf,
            )[0]
            == 409
        )
        with sqlite3.connect(fixture.RECEIPTS) as db:
            receipt = db.execute(
                "SELECT operation,state FROM actions WHERE action=?", (job["id"],)
            ).fetchone()
            assert receipt == (action, "verified")
        fixture.passed(
            f"real {action} is approved, verified, action-bound and idempotent"
        )

    for index, (action, mode, target) in enumerate(
        [
            (a, m, t)
            for a in ["start", "stop"]
            for m in ["before_prepare", "during", "after_effect"]
            for t in ["limeos-core", "limeos-containerd"]
        ],
        start=1,
    ):
        identifier = extra[index]
        if action == "start":
            fixture.docker("stop", "-t", "1", identifier)
        proposal, approval = plan(identifier, action, cookie, csrf)
        fixture.control.arm(mode, identifier)
        job, _ = queue(proposal, approval, f"{action}-{mode}-{target}", cookie, csrf)
        assert fixture.control.entered.wait(25), (
            action,
            mode,
            target,
            fixture.state(job["id"]),
        )
        pid = int(
            fixture.run("systemctl", "show", target, "-p", "MainPID", "--value").stdout
        )
        os.kill(pid, signal.SIGKILL)
        fixture.control.release.set()
        fixture.control.mode = ""
        expected = "succeeded" if target == "limeos-core" else "needs_intervention"
        fixture.eventually(
            lambda job=job, expected=expected: fixture.state(job["id"]) == expected
        )
        fixture.wait_container_ready()
        post_count = (
            0 if target == "limeos-containerd" and mode == "before_prepare" else 1
        )
        assert fixture.control.posts.get(identifier, 0) == post_count
        time.sleep(0.5)
        assert fixture.control.posts.get(identifier, 0) == post_count
        if target == "limeos-containerd" and mode != "before_prepare":
            with sqlite3.connect(fixture.RECEIPTS) as db:
                assert db.execute(
                    "SELECT operation,state FROM actions WHERE action=?", (job["id"],)
                ).fetchone() == (action, "prepared")
            with sqlite3.connect(fixture.CORE_DB) as db:
                assert (
                    db.execute(
                        "SELECT COUNT(*) FROM jobs INDEXED BY jobs_resource_lock WHERE id=? AND state IN ('running','verifying','outcome_unknown','needs_intervention')",
                        (job["id"],),
                    ).fetchone()[0]
                    == 1
                )
        fixture.passed(
            f"{action}: kill {target} {mode} retains proof and prevents replay"
        )

    for tty in [False, True]:
        command = "printf 'ready\\nAPI_KEY=synthetic-private\\n\\033[31mpassword=synthetic-hidden\\033[0m\\n<img src=x>\\n'; /bin/busybox sleep 3600"
        args = (
            ["run", "-d"]
            + (["-t"] if tty else [])
            + ["limeos-p03-fixture:local", "/bin/busybox", "sh", "-c", command]
        )
        identifier = fixture.docker(*args)
        policy["managed_containers"].append(identifier)
        policy_path.write_text(json.dumps(policy))
        fixture.restart_container()
        old_posts = sum(fixture.control.posts.values())
        status, _, logs = fixture.http(
            f"/api/v1/containers/{identifier}/logs?tail=100", cookie=cookie
        )
        assert status == 200, (status, logs)
        assert "ready" in logs["text"] and "<img src=x>" in logs["text"]
        assert (
            "synthetic-private" not in logs["text"]
            and "synthetic-hidden" not in logs["text"]
            and "\x1b" not in logs["text"]
        )
        assert len(logs["text"].encode()) <= 24 * 1024
        assert sum(fixture.control.posts.values()) == old_posts
        for tail in [0, 201, 65535]:
            assert (
                fixture.http(
                    f"/api/v1/containers/{identifier}/logs?tail={tail}", cookie=cookie
                )[0]
                == 400
            )
        fixture.passed(
            f"real {'TTY' if tty else 'multiplexed'} logs are bounded, filtered and have no effect"
        )
    assert fixture.http(f"/api/v1/containers/{ids[9]}/logs", cookie=cookie)[0] == 403
    policy["allow_container_logs"] = False
    policy_path.write_text(json.dumps(policy))
    fixture.restart_container()
    assert (
        fixture.http(f"/api/v1/containers/{identifier}/logs", cookie=cookie)[0] == 403
    )
    fixture.passed(
        "logs independently reject an unmanaged ID and a disabled log ceiling"
    )

    # A limited task can read a managed log but cannot create its own approval.
    policy["allow_container_logs"] = True
    policy_path.write_text(json.dumps(policy))
    fixture.restart_container()
    token = "c" * 64
    with sqlite3.connect(fixture.CORE_DB) as db:
        generation = db.execute("SELECT generation FROM meta").fetchone()[0]
        import pwd

        db.execute(
            "INSERT INTO task_tokens VALUES(?,?,?,?,?,?,?,?,0)",
            (
                hashlib.sha256(token.encode()).hexdigest(),
                principal["id"],
                pwd.getpwnam("limeos-assistant").pw_uid,
                "log-task",
                json.dumps(
                    [
                        {
                            "operation": "container_manage",
                            "resource": "container:" + identifier,
                        }
                    ]
                ),
                principal["grant_revision"],
                generation,
                int(time.time()) + 3600,
            ),
        )
    request = {
        "request": "read_container_logs",
        "version": 1,
        "token": token,
        "task": "log-task",
        "resource": "container:" + identifier,
        "options": {"tail": 100},
    }
    assert fixture.rpc_as("limeos-assistant", request)["response"] == "container_logs"
    request["resource"] = "container:" + ids[0]
    assert fixture.rpc_as("limeos-assistant", request)["code"] == "forbidden"
    assert (
        fixture.rpc_as("limeos-assistant", {**request, "request": "approve_container"})[
            "code"
        ]
        == "invalid_input"
    )
    fixture.passed(
        "task logs use the same resource grant and cannot acquire human approval"
    )

    with sqlite3.connect(fixture.CORE_DB) as db:
        db.execute("UPDATE users SET role='\"viewer\"' WHERE id=?", (principal["id"],))
    try:
        assert (
            fixture.http(f"/api/v1/containers/{identifier}/logs", cookie=cookie)[0]
            == 403
        )
        request["resource"] = "container:" + identifier
        assert fixture.rpc_as("limeos-assistant", request)["code"] == "forbidden"
    finally:
        with sqlite3.connect(fixture.CORE_DB) as db:
            db.execute(
                "UPDATE users SET role='\"administrator\"' WHERE id=?",
                (principal["id"],),
            )
    fixture.passed("viewer inventory authority permits neither browser nor task logs")

    identifier = fixture.docker(
        "run",
        "-d",
        "limeos-p03-fixture:local",
        "/bin/busybox",
        "sh",
        "-c",
        "printf '%70000s\\n' large; /bin/busybox sleep 3600",
    )
    policy["managed_containers"].append(identifier)
    policy_path.write_text(json.dumps(policy))
    fixture.restart_container()
    started = time.monotonic()
    status, _, logs = fixture.http(
        f"/api/v1/containers/{identifier}/logs?tail=200", cookie=cookie
    )
    assert (
        status == 200 and logs["truncated"] and len(logs["text"].encode()) <= 24 * 1024
    )
    assert time.monotonic() - started < 5
    fixture.passed("real oversized log framing clips within byte and time limits")

    for action in ["start", "stop"]:
        identifier = fixture.docker(
            "run", "-d", "limeos-p03-fixture:local", "/bin/busybox", "sleep", "3600"
        )
        if action == "start":
            fixture.docker("stop", "-t", "1", identifier)
        policy["managed_containers"].append(identifier)
        policy["allow_" + action] = False
        policy_path.write_text(json.dumps(policy))
        fixture.restart_container()
        proposal, approval = plan(identifier, action, cookie, csrf)
        denied, _ = queue(proposal, approval, "ceiling-" + action, cookie, csrf)
        fixture.eventually(
            lambda denied=denied: fixture.state(denied["id"]) == "needs_intervention"
        )
        assert fixture.control.posts.get(identifier, 0) == 0
        policy["allow_" + action] = True
        fixture.passed(
            f"{action} has an independent effect ceiling even with restart enabled"
        )

    fixture.run("apt-get", "install", "-y", "limeos-shadow=0.3.1")
    shadow_path = Path("/etc/limeos-shadow/system-policy/container.json")
    original = json.loads(shadow_path.read_text())
    for action in ["start", "stop"]:
        value = {
            **original,
            "allow_restart": False,
            "allow_start": False,
            "allow_stop": False,
            "allow_" + action: True,
            "managed_containers": [identifier],
        }
        shadow_path.write_text(json.dumps(value))
        fixture.run("systemctl", "reset-failed", "limeos-shadow-containerd")
        fixture.run("systemctl", "restart", "limeos-shadow-containerd", check=False)
        fixture.eventually(
            lambda: (
                fixture.run(
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
    fixture.run("apt-get", "remove", "-y", "limeos-shadow")
    fixture.passed(
        "shadow independently refuses start-enabled and stop-enabled ceilings"
    )

    memory = {}
    for unit in ["limeos-core", "limeos-containerd", "limeos-storaged"]:
        pid = int(
            fixture.run("systemctl", "show", unit, "-p", "MainPID", "--value").stdout
        )
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
        assert fixture.http("/api/v1/overview", cookie=cookie)[0] == 200
        elapsed.append((time.monotonic() - started) * 1000)

    result = json.loads(output.read_text())
    result["checks"] = fixture.checks
    result["lifecycle_posts"] = sum(fixture.control.posts.values())
    result["operations"] = ["restart", "start", "stop", "logs"]
    result["schemas"] = {"authority": authority_schema, "container_receipts": 3}
    result["footprint"].update(
        app_pss_kib=memory,
        combined_pss_kib=sum(memory.values()),
        overview_p95_ms=round(sorted(elapsed)[18], 3),
        overview_median_ms=round(statistics.median(elapsed), 3),
        workload="after restart/start/stop, log and interruption qualification, 20 cached overview requests",
    )
    result["upgrade"] = (
        {
            "from": "0.3.0",
            "to": package_version,
            "previous_package_sha256": {
                p.name: hashlib.sha256(p.read_bytes()).hexdigest()
                for p in Path("/opt/limeos-previous-repo/pool/main").glob("*.deb")
            },
        }
        if qualify_upgrade and Path("/opt/limeos-previous-repo").is_dir()
        else None
    )
    output.write_text(json.dumps(result, indent=2) + "\n")


if __name__ == "__main__":
    main()
