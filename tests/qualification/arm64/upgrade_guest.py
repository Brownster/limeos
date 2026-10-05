#!/usr/bin/env python3
"""Real package upgrade from a genuine older ARM64 payload to the native 0.4.2 build.

Runs only inside the disposable guest, as root:
  upgrade_guest.py OLD_DEB OLD_SHA256 REPO EXPECTED_JSON OUTPUT_JSON

The older package is the exact previously recorded artifact (its SHA-256 is
checked first); it is never rebuilt from current source with an older label.
"""

import hashlib
import http.client
import json
import os
import platform
import socket
import sqlite3
import subprocess
import sys
import time
from pathlib import Path

OLD_DEB, OLD_SHA, REPO, EXPECTED, OUTPUT = sys.argv[1:6]
OLD_DEB, REPO, EXPECTED, OUTPUT = (
    Path(OLD_DEB),
    Path(REPO),
    Path(EXPECTED),
    Path(OUTPUT),
)
VERSION = "0.4.2"
LIMEOS = Path("/usr/lib/limeos")
DB = "/var/lib/limeos/core/core.sqlite"
RESULT = {"checks": [], "observations": {}}


def save():
    OUTPUT.write_text(json.dumps(RESULT, indent=2, default=str) + "\n")


def run(*args, check=True, input=None, timeout=900):
    env = dict(os.environ, DEBIAN_FRONTEND="noninteractive")
    result = subprocess.run(
        args,
        check=False,
        input=input,
        text=True,
        capture_output=True,
        timeout=timeout,
        env=env,
    )
    if check and result.returncode != 0:
        raise RuntimeError(
            f"{args} failed ({result.returncode}): {result.stdout[-1500:]} {result.stderr[-1500:]}"
        )
    return result


def http_request(method, path, body=None, cookie=None):
    connection = http.client.HTTPConnection("127.0.0.1", 8003, timeout=15)
    headers = {"Origin": "https://localhost", "Content-Type": "application/json"}
    if cookie:
        headers["Cookie"] = cookie
    connection.request(
        method, path, json.dumps(body) if body is not None else None, headers
    )
    response = connection.getresponse()
    data = response.read()
    connection.close()
    return (
        response.status,
        dict(response.getheaders()),
        (json.loads(data) if data else None),
    )


def ready(timeout=15):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if run(str(LIMEOS / "limeosctl"), "status", check=False).returncode == 0:
            return True
        time.sleep(0.05)
    return False


def schema():
    with sqlite3.connect(f"file:{DB}?mode=ro", uri=True) as db:
        return db.execute("PRAGMA user_version").fetchone()[0], db.execute(
            "SELECT count(*) FROM users"
        ).fetchone()[0]


def units():
    listing = run(
        "systemctl", "list-unit-files", "limeos*", "--no-legend", "--no-pager"
    ).stdout
    active = {
        u: run("systemctl", "is-active", u, check=False).stdout.strip()
        for u in ["limeos-core", "limeos-containerd", "limeos-storaged"]
    }
    return {"unit_files": listing, "active": active}


def step(name, fn):
    try:
        observation = fn()
        RESULT["checks"].append(
            {"name": name, "result": "pass", "observation": observation}
        )
        print("PASS " + name, flush=True)
    except Exception as error:  # noqa: BLE001 - record every failure, then continue
        RESULT["checks"].append(
            {"name": name, "result": "fail", "error": repr(error)[-3000:]}
        )
        print("FAIL " + name + ": " + repr(error)[-400:], flush=True)
        save()
        raise SystemExit(1)
    save()


def main():
    if (
        os.getuid() != 0
        or socket.gethostname() != "limeos-p01-test"
        or platform.machine() != "aarch64"
    ):
        raise SystemExit("Disposable native aarch64 guest required")
    expected = json.loads(EXPECTED.read_text())
    state = {}

    def genuine():
        digest = hashlib.sha256(OLD_DEB.read_bytes()).hexdigest()
        assert digest == OLD_SHA, digest
        info = run(
            "dpkg-deb", "-f", str(OLD_DEB), "Package", "Version", "Architecture"
        ).stdout
        return {"sha256": digest, "control": info}

    step("older payload is the exact recorded artifact", genuine)

    def install_old():
        run("apt-get", "update", "-qq")
        run("apt-get", "install", "-y", str(OLD_DEB))
        assert ready()
        version = run("dpkg-query", "-W", "-f", "${Version}", "limeos").stdout
        issued = json.loads(run(str(LIMEOS / "limeosctl"), "bootstrap").stdout)
        state["password"] = "upgrade-" + hashlib.sha256(os.urandom(16)).hexdigest()[:20]
        run(
            str(LIMEOS / "limeosctl"),
            "enroll",
            input=json.dumps(
                {
                    "token": issued["token"],
                    "username": "alice",
                    "password": state["password"],
                }
            ),
        )
        code, headers, _ = http_request(
            "POST",
            "/api/v1/auth/login",
            {"username": "alice", "password": state["password"]},
        )
        assert code == 200, code
        state["cookie"] = headers["set-cookie"].split(";", 1)[0]
        assert (
            http_request("GET", "/api/v1/auth/session", cookie=state["cookie"])[0]
            == 200
        )
        state["old_schema"] = schema()
        return {
            "installed_version": version,
            "schema_user_version_and_users": state["old_schema"],
            **units(),
        }

    step(
        "older package installs, enrolls and holds an authenticated session",
        install_old,
    )

    def upgrade():
        run(
            "install",
            "-m",
            "0644",
            str(REPO / "limeos-archive-keyring.gpg"),
            "/usr/share/keyrings/limeos-test.gpg",
        )
        Path("/etc/apt/sources.list.d/limeos-test.list").write_text(
            f"deb [signed-by=/usr/share/keyrings/limeos-test.gpg] file:{REPO} stable main\n"
        )
        run("apt-get", "update", "-qq")
        started = time.monotonic()
        output = run(
            "apt-get",
            "install",
            "-y",
            "-o",
            "Dpkg::Options::=--force-confdef",
            "-o",
            "Dpkg::Options::=--force-confold",
            f"limeos={VERSION}",
        )
        seconds = round(time.monotonic() - started, 1)
        assert ready(20)
        identity = run(
            "dpkg-query", "-W", "-f", "${Package} ${Version} ${Architecture}", "limeos"
        ).stdout
        assert identity == f"limeos {VERSION} arm64", identity
        verify = run("dpkg", "--verify", "limeos", check=False)
        installed = {
            n: hashlib.sha256((LIMEOS / n).read_bytes()).hexdigest()
            for n in expected["binaries"]
        }
        assert all(
            installed[n] == expected["binaries"][n]["sha256"] for n in installed
        ), installed
        return {
            "identity": identity,
            "apt_seconds": seconds,
            "dpkg_verify": verify.stdout,
            "apt_tail": output.stdout[-1500:],
            "installed_sha256": installed,
        }

    step("apt upgrades the genuine older package to the native 0.4.2 build", upgrade)

    def preserved():
        new_schema = schema()
        assert new_schema[0] == 6, new_schema
        assert new_schema[1] == state["old_schema"][1], (
            state["old_schema"],
            new_schema,
        )
        session = http_request("GET", "/api/v1/auth/session", cookie=state["cookie"])[0]
        assert session == 200, session
        code = http_request(
            "POST",
            "/api/v1/auth/login",
            {"username": "alice", "password": state["password"]},
        )[0]
        assert code == 200, code
        assert http_request("GET", "/api/v1/overview", cookie=state["cookie"])[0] == 200
        current = units()
        assert all(v == "active" for v in current["active"].values()), current
        return {
            "schema_before": state["old_schema"],
            "schema_after": new_schema,
            "session_after_upgrade": session,
            **current,
        }

    step(
        "upgrade migrates authority to schema 6 and preserves users and the live session",
        preserved,
    )
    RESULT["summary"] = "pass"
    save()


if __name__ == "__main__":
    main()
