#!/usr/bin/env python3
"""Qualify protected Compose previews after the complete lifecycle VM suite."""

import hashlib
import json
import os
import pwd
import socket
import sqlite3
import sys
import tarfile
import time
from pathlib import Path

sys.path.insert(0, "/root/build/source/tests/privileged_vm")
import p03_guest as fixture
import p03_lifecycle_guest as lifecycle

VERSION = "0.3.2"
BASE = "/api/v1/compose/plans"
CATALOG = Path("/etc/limeos/compose-catalog.json")


def digest(value):
    return hashlib.sha256(value.encode()).hexdigest()


def restart_core():
    # Root fixture setup, never an application retry of an interrupted effect.
    fixture.run("systemctl", "reset-failed", "limeos-core")
    fixture.run("systemctl", "restart", "limeos-core")
    fixture.eventually(lambda: fixture.http("/api/v1/health")[0] == 200)


def upgrade(candidate):
    previous = Path("/opt/limeos-previous-repo")
    if not previous.is_dir():
        return None
    lifecycle.repository(previous)
    fixture.run("apt-get", "install", "-y", "limeos=0.3.1")
    cookie, csrf, _ = fixture.enroll()
    rootfs = Path("/root/compose-upgrade-rootfs.tar")
    with tarfile.open(rootfs, "w") as archive:
        archive.add("/bin/busybox", arcname="bin/busybox")
    fixture.run("docker", "import", str(rootfs), "limeos-compose-upgrade:local")
    identifier = fixture.run(
        "docker",
        "run",
        "-d",
        "limeos-compose-upgrade:local",
        "/bin/busybox",
        "sleep",
        "3600",
    ).stdout.strip()
    policy_path = Path("/etc/limeos/system-policy/container.json")
    policy = json.loads(policy_path.read_text())
    policy.update(allow_restart=True, managed_containers=[identifier])
    policy_path.write_text(json.dumps(policy))
    fixture.restart_container()
    proposal, approval = fixture.plan(identifier, cookie, csrf)
    old_job, old_body = fixture.queue(
        proposal, approval, "compose-upgrade-verified", cookie, csrf
    )
    fixture.eventually(lambda: fixture.state(old_job["id"]) == "succeeded")
    pending, token = fixture.plan(identifier, cookie, csrf)
    with sqlite3.connect(fixture.CORE_DB) as db:
        assert db.execute("PRAGMA user_version").fetchone()[0] == 4
    with sqlite3.connect(fixture.RECEIPTS) as db:
        assert db.execute("PRAGMA user_version").fetchone()[0] == 3
    lifecycle.repository(candidate)
    fixture.run("apt-get", "install", "-y", "limeos=" + VERSION)
    fixture.eventually(
        lambda: fixture.http("/api/v1/overview", cookie=cookie)[0] == 200
    )
    assert (
        fixture.http(fixture.BASE + "/jobs", "POST", old_body, cookie, csrf)[2]["id"]
        == old_job["id"]
    )
    new_job, _ = fixture.queue(pending, token, "compose-upgrade-pending", cookie, csrf)
    fixture.eventually(lambda: fixture.state(new_job["id"]) == "succeeded")
    with sqlite3.connect(fixture.CORE_DB) as db:
        assert db.execute("PRAGMA user_version").fetchone()[0] == 5
        assert db.execute("SELECT COUNT(*) FROM container_results").fetchone()[0] == 2
    with sqlite3.connect(fixture.RECEIPTS) as db:
        assert db.execute("PRAGMA user_version").fetchone()[0] == 3
        assert (
            db.execute(
                "SELECT COUNT(*) FROM actions WHERE state='verified'"
            ).fetchone()[0]
            == 2
        )
    policy.update(allow_restart=False, managed_containers=[])
    policy_path.write_text(json.dumps(policy))
    fixture.restart_container()
    fixture.run("docker", "rm", "-f", identifier)
    fixture.passed(
        "real 0.3.1 to 0.3.2 upgrade preserves sessions, pending approvals and verified container receipts"
    )
    return {
        "from": "0.3.1",
        "to": VERSION,
        "previous_package_sha256": {
            p.name: hashlib.sha256(p.read_bytes()).hexdigest()
            for p in (previous / "pool/main").glob("*.deb")
        },
    }


def authority(principal, role, scopes):
    """Offline root-only identity fixtures, without an HTTP authority editor."""
    fixture.run("systemctl", "stop", "limeos-core")
    with sqlite3.connect(fixture.CORE_DB) as db:
        db.execute(
            "UPDATE users SET role=?,grant_revision=grant_revision+1 WHERE id=?",
            (json.dumps(role), principal),
        )
        db.execute("DELETE FROM grants WHERE principal=?", (principal,))
        for scope in scopes:
            db.execute(
                "INSERT INTO grants VALUES(?,?,?)",
                (principal, json.dumps(scope["operation"]), scope["resource"]),
            )
    restart_core()
    return fixture.enroll()[:2]


def task(principal, scopes):
    token = os.urandom(32).hex()
    with sqlite3.connect(fixture.CORE_DB) as db:
        revision = db.execute(
            "SELECT grant_revision FROM users WHERE id=?", (principal,)
        ).fetchone()[0]
        generation = db.execute("SELECT generation FROM meta").fetchone()[0]
        db.execute(
            "INSERT INTO task_tokens VALUES(?,?,?,?,?,?,?,?,0)",
            (
                digest(token),
                principal,
                pwd.getpwnam("limeos-assistant").pw_uid,
                "compose-preview",
                json.dumps(scopes),
                revision,
                generation,
                int(time.time()) + 60,
            ),
        )
    return token


def preview(cookie, csrf, template="standard"):
    return fixture.http(
        BASE, "POST", {"stack": "media", "template": template}, cookie, csrf
    )


def invalid_catalog(value, message):
    CATALOG.write_bytes(value)
    fixture.run("systemctl", "stop", "limeos-core")
    fixture.run("systemctl", "reset-failed", "limeos-core")
    fixture.run("systemctl", "start", "limeos-core", check=False)
    fixture.eventually(
        lambda: (
            fixture.run(
                "systemctl", "show", "limeos-core", "-p", "Result", "--value"
            ).stdout.strip()
            == "exit-code"
        )
    )
    assert CATALOG.read_bytes() == value
    fixture.run("systemctl", "stop", "limeos-core")
    fixture.passed(message)


def main(package_version="0.3.2", authority_schema=5, qualify_upgrade=True):
    global VERSION
    VERSION = package_version
    if os.getuid() != 0 or socket.gethostname() != "limeos-p01-test":
        raise SystemExit("Disposable guest required")
    candidate, output = map(Path, sys.argv[1:])
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
    upgrade_result = upgrade(candidate) if qualify_upgrade else None
    lifecycle.main(
        package_version=VERSION,
        authority_schema=authority_schema,
        qualify_upgrade=False,
    )
    cookie, csrf, principal = fixture.enroll()
    assert preview(cookie, csrf)[0] == 503
    fixture.passed(
        "a missing optional catalog disables previews without affecting the core or Python services"
    )

    catalog = json.loads(Path("/root/compose-catalog-fixture.json").read_text())
    source_dir = Path("/var/lib/limeos-compose-fixture")
    source_dir.mkdir(mode=0o755)
    operator_file = source_dir / "compose.yaml"
    original = b"# Operator formatting and anchors must survive\nx-defaults: &defaults\n  restart: unless-stopped\nservices:\n  api:\n    <<: *defaults\n    image: example.invalid/api:operator\n"
    operator_file.write_bytes(original)
    catalog["stacks"][0]["operator_files"][0]["sha256"] = hashlib.sha256(
        original
    ).hexdigest()
    CATALOG.write_text(json.dumps(catalog))
    CATALOG.chmod(0o644)
    restart_core()
    base = {"operation": "deployment_manage", "resource": "stack:media"}
    cookie, csrf = authority(principal["id"], "operator", [base])
    status, _, proposal = preview(cookie, csrf)
    assert status == 201, (status, proposal)
    changes = proposal["plan"]["impact"]["services"]
    assert [s["name"] for s in changes] == ["api", "retired", "worker"]
    assert changes[1]["after"] is None and changes[2]["before"] is None
    assert changes[0]["after"]["user"] == {"uid": 1000, "gid": 1000}
    assert proposal["plan"]["managed_file"] == "limeos.override.json"
    assert (
        proposal["plan"]["operator_files"][0]["sha256"]
        == hashlib.sha256(original).hexdigest()
    )
    assert operator_file.read_bytes() == original
    fixture.passed(
        "normal scoped preview exposes added/removed/changed services, numeric ownership and unchanged operator fingerprints"
    )

    identifier = proposal["plan"]["id"]
    with sqlite3.connect(fixture.CORE_DB) as db:
        before = (
            db.execute("SELECT COUNT(*) FROM events").fetchone(),
            db.execute(
                "SELECT * FROM compose_plans WHERE id=?", (identifier,)
            ).fetchone(),
        )
    for _ in range(20):
        assert fixture.http(BASE + "/" + identifier, cookie=cookie)[2] == proposal
    with sqlite3.connect(fixture.CORE_DB) as db:
        after = (
            db.execute("SELECT COUNT(*) FROM events").fetchone(),
            db.execute(
                "SELECT * FROM compose_plans WHERE id=?", (identifier,)
            ).fetchone(),
        )
    assert before == after
    for path in [
        BASE,
        BASE + f"/{identifier}/approval",
        BASE + f"/{identifier}/cancel",
    ]:
        assert fixture.http(path, cookie=cookie)[0] == 405
    assert (
        fixture.http(
            BASE + f"/{identifier}/approval",
            "POST",
            {"digest": proposal["digest"]},
            cookie,
            "a" * 64,
        )[0]
        == 403
    )
    assert (
        fixture.http(
            BASE + f"/{identifier}/approval", "POST", {"digest": "a" * 64}, cookie, csrf
        )[0]
        == 409
    )
    fixture.passed(
        "GET never creates or approves a preview and repeated reads do not write authority; CSRF and exact digest are enforced"
    )
    status, _, approval = fixture.http(
        BASE + f"/{identifier}/approval",
        "POST",
        {"digest": proposal["digest"]},
        cookie,
        csrf,
    )
    assert status == 200
    with sqlite3.connect(fixture.CORE_DB) as db:
        hashed = db.execute(
            "SELECT approval_digest FROM compose_plans WHERE id=?", (identifier,)
        ).fetchone()[0]
    assert hashed == digest(approval["token"])
    restart_core()
    assert fixture.http(BASE + "/" + identifier, cookie=cookie)[2] == proposal
    with sqlite3.connect(fixture.CORE_DB) as db:
        assert (
            db.execute(
                "SELECT approval_digest FROM compose_plans WHERE id=?", (identifier,)
            ).fetchone()[0]
            == hashed
        )
    assert fixture.http("/api/v1/compose/jobs", "POST", {}, cookie, csrf)[0] == 404
    assert (
        fixture.http(
            fixture.BASE + "/jobs",
            "POST",
            {
                "proposal": proposal,
                "approval": approval["token"],
                "idempotency_key": "wrong-domain",
            },
            cookie,
            csrf,
        )[0]
        == 400
    )
    fixture.passed(
        "hashed preview approval survives restart and cannot become a Docker/container job"
    )

    token = task(principal["id"], [base])
    request = {
        "request": "propose_compose",
        "version": 1,
        "token": token,
        "task": "compose-preview",
        "selection": {"stack": "media", "template": "standard"},
    }
    model = fixture.rpc_as("limeos-assistant", request)
    assert model["response"] == "compose_plan", model
    assert model["plan"]["desired"] == proposal["plan"]["desired"]
    assert model["plan"]["impact"] == proposal["plan"]["impact"]
    assert fixture.rpc_as("limeos-core", request)["code"] == "expired"
    assert (
        fixture.http(
            BASE + f"/{identifier}/approval",
            "POST",
            {"digest": proposal["digest"]},
            "__Host-limeos=" + token,
            csrf,
        )[0]
        == 401
    )
    assert (
        fixture.rpc_as(
            "limeos-assistant",
            {
                "request": "approve_compose",
                "version": 1,
                "token": token,
                "id": identifier,
                "digest": proposal["digest"],
            },
        )["code"]
        == "invalid_input"
    )
    fixture.passed(
        "framed task proposals share browser policy and content; kernel UID and human-only approval prevent authority substitution"
    )

    assert preview(cookie, csrf, "elevated")[0] == 403
    cookie, csrf = authority(
        principal["id"],
        "administrator",
        [base, {"operation": "deployment_elevated", "resource": "*"}],
    )
    assert preview(cookie, csrf, "elevated")[0] == 403
    elevated_project = catalog["stacks"][0]["templates"][1]["project"]
    template_digest = digest(json.dumps(elevated_project, separators=(",", ":")))
    elevated = {
        "operation": "deployment_elevated",
        "resource": f"stack:media:template:elevated:{template_digest}",
    }
    cookie, csrf = authority(principal["id"], "administrator", [base, elevated])
    status, _, elevated_proposal = preview(cookie, csrf, "elevated")
    assert status == 201, (status, elevated_proposal)
    assert elevated_proposal["plan"]["template_digest"] == template_digest
    assert elevated_proposal["plan"]["impact"]["elevated"] == [
        "privileged",
        "host_mount",
        "device",
        "docker_socket",
        "host_network",
    ]
    token = task(principal["id"], [base])
    request.update(token=token, selection={"stack": "media", "template": "elevated"})
    assert fixture.rpc_as("limeos-assistant", request)["code"] == "forbidden"
    request["token"] = task(principal["id"], [base, elevated])
    assert fixture.rpc_as("limeos-assistant", request)["response"] == "compose_plan"
    fixture.passed(
        "all five elevations require the exact stack/template/content grant; wildcard admins and narrowed tasks cannot broaden it"
    )

    status, _, proposal = preview(cookie, csrf)
    assert status == 201
    identifier = proposal["plan"]["id"]
    changed = json.loads(json.dumps(catalog))
    changed["stacks"][0]["operator_files"][0]["sha256"] = "2" * 64
    CATALOG.write_text(json.dumps(changed))
    restart_core()
    assert (
        fixture.http(
            BASE + f"/{identifier}/approval",
            "POST",
            {"digest": proposal["digest"]},
            cookie,
            csrf,
        )[0]
        == 409
    )
    CATALOG.write_text(json.dumps(catalog))
    restart_core()
    assert (
        fixture.http(BASE + f"/{identifier}/cancel", "POST", {}, cookie, csrf)[0] == 204
    )
    assert fixture.http(BASE + "/" + identifier, cookie=cookie)[0] == 404
    fixture.passed(
        "changed catalog fingerprints block old approvals and cancellation removes access to the preview"
    )
    cookie, csrf = authority(
        principal["id"],
        "media_requester",
        [base, elevated, {"operation": "media_request", "resource": "library"}],
    )
    assert preview(cookie, csrf)[0] == 403
    fixture.passed(
        "media-requester roles cannot deploy even when supplied deployment scopes"
    )

    valid = json.dumps(catalog).encode()
    for value, message in [
        (b"{broken", "corrupt catalog fails closed and is preserved"),
        (
            valid + b" " * 65536,
            "oversized catalog fails before unlimited input allocation",
        ),
    ]:
        invalid_catalog(value, message)
    ambiguous = json.loads(json.dumps(catalog))
    ambiguous["stacks"][0]["operator_files"].append(
        {"name": "docker-compose.yml", "sha256": "2" * 64}
    )
    invalid_catalog(
        json.dumps(ambiguous).encode(),
        "ambiguous Compose filenames are rejected without editing operator YAML",
    )
    unsupported = json.loads(json.dumps(catalog))
    unsupported["stacks"][0]["templates"][0]["project"]["services"][0]["cap_add"] = [
        "SYS_ADMIN"
    ]
    invalid_catalog(
        json.dumps(unsupported).encode(),
        "unsupported privilege fields cannot escape the typed schema",
    )
    CATALOG.write_bytes(valid)
    CATALOG.chmod(0o666)
    invalid_catalog(valid, "world-writable catalog is refused")
    CATALOG.chmod(0o644)
    os.chown(CATALOG, pwd.getpwnam("limeos-core").pw_uid, 0)
    invalid_catalog(valid, "service-owned catalog cannot become planning authority")
    os.chown(CATALOG, 0, 0)
    parent_mode = CATALOG.parent.stat().st_mode & 0o777
    CATALOG.parent.chmod(0o777)
    invalid_catalog(valid, "root-owned catalog under a writable parent is refused")
    CATALOG.parent.chmod(parent_mode)
    target = CATALOG.with_name("compose-catalog-source.json")
    target.write_bytes(valid)
    CATALOG.unlink()
    CATALOG.symlink_to(target)
    fixture.run("systemctl", "reset-failed", "limeos-core")
    fixture.run("systemctl", "start", "limeos-core", check=False)
    fixture.eventually(
        lambda: (
            fixture.run(
                "systemctl", "show", "limeos-core", "-p", "Result", "--value"
            ).stdout.strip()
            == "exit-code"
        )
    )
    assert CATALOG.is_symlink() and target.read_bytes() == valid
    fixture.run("systemctl", "stop", "limeos-core")
    CATALOG.unlink()
    CATALOG.write_bytes(valid)
    CATALOG.chmod(0o644)
    restart_core()
    fixture.passed("symlink catalogs are refused and their targets preserved")
    assert operator_file.read_bytes() == original
    assert not (source_dir / "limeos.override.json").exists()
    fixture.passed(
        "all preview scenarios leave operator files and stack directories byte-for-byte untouched"
    )

    result = json.loads(output.read_text())
    result["checks"] = fixture.checks
    result["operations"].append("compose.preview")
    result["compose"] = {
        "catalog_fixture_sha256": hashlib.sha256(
            Path("/root/compose-catalog-fixture.json").read_bytes()
        ).hexdigest(),
        "operator_file_sha256": hashlib.sha256(original).hexdigest(),
        "host_effects": 0,
        "catalog_source": "synthetic protected planning snapshot; no reference host data",
        "deployment_executable": False,
    }
    result["upgrade"] = upgrade_result
    output.write_text(json.dumps(result, indent=2) + "\n")


if __name__ == "__main__":
    main()
