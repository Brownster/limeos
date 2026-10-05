#!/usr/bin/env python3
"""Installed guided storage previews; disposable VM disks and identities only."""

import copy
import hashlib
import http.client as http_client
import json
import os
import sqlite3
import sys
import tempfile
import time
from pathlib import Path

import p04_storage_guest as readiness

VERSION = "0.4.1"
AUTHORITY_SCHEMA = 6
DB = "/var/lib/limeos/core/core.sqlite"
BASE = "/api/v1/storage"
run = readiness.run


def http(
    path, method="GET", body=None, cookie=None, csrf=None, origin="https://localhost"
):
    connection = http_client.HTTPConnection("127.0.0.1", 8003, timeout=12)
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


def contract(profile, devices):
    return {
        "schema_version": "1",
        "profile": profile,
        "media_identity": {"uid": 1000, "gid": 1000},
        "locations": {
            "media_host": "/mnt/storage/media"
            if profile == "protected_pool"
            else "/mnt/data/media",
            "downloads_host": "/mnt/downloads"
            if profile == "separate_downloads"
            else "/mnt/storage/downloads"
            if profile == "protected_pool"
            else "/mnt/data/downloads",
            "application_config_host": "/var/lib/limeos/apps",
            "backup_host": "/mnt/backup/limeos",
            "media_container": "/data/media",
            "downloads_container": "/data/downloads",
            "config_container": "/config",
        },
        "devices": devices,
    }


def main():
    if (
        os.geteuid() != 0
        or Path("/etc/hostname").read_text().strip() != "limeos-p01-test"
    ):
        raise SystemExit("Refusing outside disposable VM")
    readiness.VERSION = VERSION
    readiness.main()
    output = Path(sys.argv[2])
    evidence = json.loads(output.read_text())
    checks = evidence["passed"]

    def passed(name):
        checks.append(name)
        print("PASS " + name, flush=True)

    repo = Path(sys.argv[1])
    shadow_package = next(repo.rglob(f"limeos-shadow_{VERSION}_amd64.deb"))
    with tempfile.TemporaryDirectory(prefix="limeos-shadow-payload-") as temporary:
        run("dpkg-deb", "-x", str(shadow_package), temporary)
        shadow_unit = (
            Path(temporary) / "lib/systemd/system/limeos-shadow-storage-reader.service"
        ).read_text()
    for prefix, unit in [
        ("limeos", run("systemctl", "cat", "limeos-storage-reader").stdout),
        ("limeos-shadow", shadow_unit),
    ]:
        assert (
            "[Install]" not in unit
            and "PrivateMounts=no" in unit
            and "CapabilityBoundingSet=\n" in unit
        )
        assert (
            prefix + "/limeos-executor" in unit
            and prefix + "/system-policy/storage.json" in unit
        )
    assert (
        run(
            "systemctl", "is-enabled", "limeos-storage-reader", check=False
        ).stdout.strip()
        == "static"
    )
    assert (
        run("systemctl", "is-active", "limeos-storage-reader", check=False).returncode
        != 0
    )
    with sqlite3.connect(DB) as db:
        assert db.execute("PRAGMA user_version").fetchone()[0] == AUTHORITY_SCHEMA
        original_jobs = db.execute(
            "SELECT id,intent,digest FROM jobs ORDER BY id"
        ).fetchall()
        db.execute(
            "INSERT INTO grants VALUES(?,?,?)",
            ("legacy-0", '"storage_manage"', "storage:configuration"),
        )
    passwords = json.loads(Path("/root/werkzeug-hashes.json").read_text())
    code, headers, session = http(
        "/api/v1/auth/login",
        "POST",
        {"username": "legacy-0", "password": passwords[0]["password"]},
    )
    assert code == 200
    cookie, csrf = headers["set-cookie"].split(";", 1)[0], session["csrf_token"]
    assert http(BASE + "/inventory", cookie=cookie)[0] == 503
    run("systemctl", "start", "limeos-storage-reader")
    pid = run(
        "systemctl", "show", "limeos-storage-reader", "-p", "MainPID", "--value"
    ).stdout.strip()
    status = Path(f"/proc/{pid}/status").read_text()
    assert "CapEff:\t0000000000000000" in status and "NoNewPrivs:\t1" in status
    passed(
        f"schema {AUTHORITY_SCHEMA}, dormant standard/shadow reader payload and installed empty-capability host reader"
    )

    fstab = Path("/etc/fstab")
    original = fstab.read_bytes()
    sentinels = {
        name: hashlib.sha256(
            (Path("/mnt") / name / "sentinel").read_bytes()
        ).hexdigest()
        for name in ["data", "data2", "downloads", "parity", "backup"]
    }

    def inventory():
        code, _, value = http(BASE + "/inventory", cookie=cookie)
        assert code == 200, value
        return value

    def propose(value, selection):
        return http(
            BASE + "/plans",
            "POST",
            {
                "contract": selection,
                "inventory_digest": value["digest"],
                "timeout_seconds": 10,
            },
            cookie,
            csrf,
        )

    def approve(plan):
        return http(
            BASE + "/plans/" + plan["plan"]["id"] + "/approval",
            "POST",
            {"digest": plan["digest"]},
            cookie,
            csrf,
        )

    assignments = [
        readiness.assignment(name, "/dev/" + device, "/mnt/" + name, serial, role=role)
        for name, device, serial, role in [
            ("data", "vdb1", "limeos-test-0", "data"),
            ("data2", "vdb2", "limeos-test-0", "data"),
            ("downloads", "vdc1", "limeos-test-1", "downloads"),
            ("parity", "vdd1", "limeos-test-2", "parity"),
            ("backup", "vde1", "limeos-test-3", "config_backup"),
        ]
    ]
    single = contract("single_disk", assignments[:1])
    try:
        fstab.write_bytes(
            original
            + b"//fixture/share /mnt/remote cifs username=private-user,password=private-secret 0 0\n"
        )
        snapshot = inventory()
        raw = json.dumps(snapshot)
        assert (
            "private-user" not in raw
            and "private-secret" not in raw
            and "fixture/share" not in raw
        )
        assert any(d["boot_backing"] for d in snapshot["inventory"]["devices"])
        assert len(snapshot["inventory"]["devices"]) >= 6
        cli = json.loads(run(readiness.EXECUTOR, "storage-inventory").stdout)
        assert cli == snapshot["inventory"]
        passed(
            "fresh raw disk inventory agrees across CLI/RPC/HTTP and excludes fstab credentials"
        )

        plans = []
        before = fstab.read_bytes()
        for profile, devices in [
            ("single_disk", assignments[:1]),
            ("separate_downloads", [assignments[0], assignments[2]]),
            (
                "protected_pool",
                [assignments[0], assignments[1], assignments[3], assignments[4]],
            ),
        ]:
            code, _, plan = propose(snapshot, contract(profile, devices))
            assert code == 201, plan
            plans.append(plan)
            assert len(plan["plan"]["readiness"]["devices"]) == len(devices)
            assert (
                "nodev,nosuid,x-systemd.device-timeout=10s,x-systemd.mount-timeout=10s"
                in plan["plan"]["managed_fstab"]
            )
            assert "private-secret" not in json.dumps(plan)
            assert fstab.read_bytes() == before
        passed(
            "three guided profiles persist deterministic UUID/fstab/readiness previews without disk or fstab writes"
        )

        plan = plans[0]
        run("systemctl", "restart", "limeos-core")
        # Status polls do not authenticate or create authority.
        for _ in range(30):
            if run("/usr/lib/limeos/limeosctl", "status", check=False).returncode == 0:
                break
            time.sleep(0.1)
        assert http(BASE + "/plans/" + plan["plan"]["id"], cookie=cookie)[2] == plan
        code, _, approval = approve(plan)
        assert code == 200, approval
        with sqlite3.connect(DB) as db:
            assert (
                db.execute(
                    "SELECT approval_digest FROM storage_plans WHERE id=?",
                    (plan["plan"]["id"],),
                ).fetchone()[0]
                == hashlib.sha256(approval["token"].encode()).hexdigest()
            )
            assert (
                db.execute("SELECT id,intent,digest FROM jobs ORDER BY id").fetchall()
                == original_jobs
            )
        passed(
            "restart preserves human session and preview; approval is hashed and creates no executable job"
        )

        fstab.write_bytes(before + b"# operator edit during review\n")
        assert approve(plan)[0] == 409
        assert http(BASE + "/plans/" + plan["plan"]["id"], cookie=cookie)[0] == 200
        assert (
            http(
                BASE + "/plans/" + plan["plan"]["id"] + "/cancel",
                "POST",
                cookie=cookie,
                csrf=csrf,
            )[0]
            == 204
        )
        assert http(BASE + "/plans/" + plan["plan"]["id"], cookie=cookie)[0] == 404
        assert propose(snapshot, single)[0] == 409
        passed(
            "operator fstab edit invalidates stale proposal/approval while review and withdrawal remain available"
        )

        fstab.write_bytes(before)
        boot = next(
            d
            for d in inventory()["inventory"]["devices"]
            if d["boot_backing"] and d["filesystem"] == "ext4"
        )
        forbidden = copy.deepcopy(single)
        forbidden["devices"][0].update(filesystem_uuid=boot["filesystem_uuid"])
        forbidden["devices"][0].pop("serial")
        assert propose(inventory(), forbidden)[0] == 409
        wrong = copy.deepcopy(single)
        wrong["devices"][0]["serial"] = "replacement"
        assert propose(inventory(), wrong)[0] == 409
        fstab.write_bytes(
            before
            + f"UUID={assignments[0]['filesystem_uuid']} /mnt/operator ext4 defaults 0 2\n".encode()
        )
        assert propose(inventory(), single)[0] == 409
        assert fstab.read_bytes().endswith(b"/mnt/operator ext4 defaults 0 2\n")
        passed(
            "boot backing, changed serial and unmanaged fstab ownership conflict are refused without overwriting the operator entry"
        )

        fstab.write_bytes(before)
        code, _, plan = propose(inventory(), single)
        assert code == 201
        run("umount", "/mnt/data")
        assert approve(plan)[0] == 409
        unmounted = inventory()
        target = next(
            d
            for d in unmounted["inventory"]["devices"]
            if d["filesystem_uuid"] == assignments[0]["filesystem_uuid"]
        )
        assert target["mounts"] == []
        assert propose(unmounted, single)[0] == 201
        original_uuid = readiness.uuid("/dev/vdb1")
        run("umount", "/mnt/downloads")
        # The readiness suite previously relabeled/remounted this synthetic
        # filesystem. tune2fs requires a fresh check before another UUID change.
        checked = run("e2fsck", "-f", "-p", "/dev/vdc1", check=False)
        assert checked.returncode in [0, 1], (
            checked.returncode,
            checked.stdout,
            checked.stderr,
        )
        changed_uuid = run("tune2fs", "-U", original_uuid, "/dev/vdc1", check=False)
        assert changed_uuid.returncode == 0, (
            changed_uuid.returncode,
            changed_uuid.stdout,
            changed_uuid.stderr,
        )
        assert propose(inventory(), single)[0] == 409
        checked = run("e2fsck", "-f", "-p", "/dev/vdc1", check=False)
        assert checked.returncode in [0, 1], (
            checked.returncode,
            checked.stdout,
            checked.stderr,
        )
        run("tune2fs", "-U", assignments[2]["filesystem_uuid"], "/dev/vdc1")
        run("mount", "/dev/vdb1", "/mnt/data")
        run("mount", "/dev/vdc1", "/mnt/downloads")
        passed(
            "mount loss prevents old approval; unmounted filesystems can be selected and duplicate raw UUIDs cannot"
        )

        for suffix in [
            "/plans",
            "/plans/" + plan["plan"]["id"] + "/approval",
            "/plans/" + plan["plan"]["id"] + "/cancel",
        ]:
            assert http(BASE + suffix, cookie=cookie)[0] == 405
            assert (
                http(
                    BASE + suffix, "POST", {}, cookie, csrf, "https://foreign.invalid"
                )[0]
                == 403
            )
            assert http(BASE + suffix, "POST", {}, cookie, "0" * 64)[0] == 403
        assert http(BASE + "/jobs", "POST", {}, cookie, csrf)[0] == 404
        assert http(BASE + "/inventory")[0] == 401
        with sqlite3.connect(DB) as db:
            db.execute("DELETE FROM grants WHERE principal='legacy-0'")
        assert http(BASE + "/inventory", cookie=cookie)[0] == 403
        malformed = before + b"# BEGIN LIMEOS STORAGE V1\n"
        fstab.write_bytes(malformed)
        assert run(readiness.EXECUTOR, "storage-inventory", check=False).returncode != 0
        assert fstab.read_bytes() == malformed
        passed(
            "storage routes require current grants, Origin/CSRF and POST; malformed fstab remains unchanged and execution is unavailable"
        )
    finally:
        fstab.write_bytes(original)
    for name, expected in sentinels.items():
        assert (
            hashlib.sha256((Path("/mnt") / name / "sentinel").read_bytes()).hexdigest()
            == expected
        )
    assert not Path("/mnt/data/media").exists() and not Path("/mnt/storage").exists()
    passed(
        "all synthetic data and original fstab survive preview, approval, restart and refusal scenarios"
    )
    run("/var/lib/dpkg/info/limeos.prerm", "upgrade")
    assert (
        run("systemctl", "is-active", "limeos-storage-reader", check=False).returncode
        != 0
    )
    passed(
        "package pre-removal stops a manually started storage reader before replacing binaries"
    )
    evidence.update(
        scope="installed guided assignment previews and fresh read-only storage evidence; no mount/fstab effect registered",
        reader_payload_sha256=hashlib.sha256(
            Path(readiness.EXECUTOR).read_bytes()
        ).hexdigest(),
    )
    evidence["limitations"].extend(
        [
            "Guided UI, safe target preparation and executable storage receipts/locks pending",
            "Shadow reader payload checked, shadow services not installed in this run",
            "No genuine old-to-new package upgrade in this run",
        ]
    )
    output.write_text(json.dumps(evidence, indent=2) + "\n")


if __name__ == "__main__":
    main()
