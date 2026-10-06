#!/usr/bin/env python3
"""Approved empty target jobs and genuine schema-7 upgrade, disposable disks only."""

import copy
import hashlib
import json
import os
import signal
import sqlite3
import sys
import tempfile
import time
from pathlib import Path

import p03_guest as containers
import p04_locks_guest as locks
import p04_planning_guest as planning
import p04_storage_guest as storage
import p04_targets_guest as targets
import p04_targets_package_guest as package_tests

VERSION = "0.4.4"
BASE = "/api/v1/storage/targets"
POLICY = Path("/etc/limeos/system-policy/storage-targets.json")
STATE = Path("/var/lib/limeos/executors/storage")
EXECUTOR = targets.EXECUTOR
run = planning.run
http = planning.http
eventually = containers.eventually


def main():
    if (
        os.geteuid() != 0
        or Path("/etc/hostname").read_text().strip() != "limeos-p01-test"
    ):
        raise SystemExit("Refusing outside disposable VM")
    repo, output = map(Path, sys.argv[1:])
    locks.VERSION = VERSION
    upgrade = locks.upgrade(
        repo,
        previous_version="0.4.3",
        previous_schema=7,
        authority_schema=8,
        previous_package_sha256="c5e49b2117a19d0983b5afb9a3ea756f479c2ab9dc67be7001758265737204b6",
        previous_core_sha256="99c2c8a2a19919e093476e33fbaae9fc0d4da8559bfa8b5a2151135b23243c33",
    )
    locks.main(VERSION, authority_schema=8, qualify_upgrade=False)
    evidence = json.loads(output.read_text())

    def passed(name):
        evidence["passed"].append(name)
        print("PASS " + name, flush=True)

    if upgrade:
        passed(
            "genuine frozen 0.4.3 to 0.4.4 upgrade preserves sessions, canonical approval bytes, verified receipts and four complete active claim sets; old replay and pending approved restart pass"
        )
    # Remove only the private no-effect HealthProbe fixtures left by the claim
    # tests. Real dispatched operation records never use this cleanup path.
    run("systemctl", "stop", "limeos-core")
    with sqlite3.connect(planning.DB) as db:
        db.execute(
            "UPDATE jobs SET state='canceled' WHERE json_extract(intent,'$.operation')='health_probe' AND resource LIKE 'fixture:%' AND state IN ('queued','running','verifying','outcome_unknown','needs_intervention')"
        )
    run("systemctl", "start", "limeos-core")
    eventually(
        lambda: run("/usr/lib/limeos/limeosctl", "status", check=False).returncode == 0
    )
    passwords = json.loads(Path("/root/werkzeug-hashes.json").read_text())
    code, headers, session = http(
        "/api/v1/auth/login",
        "POST",
        {"username": "legacy-0", "password": passwords[0]["password"]},
    )
    assert code == 200
    cookie = headers["set-cookie"].split(";", 1)[0]
    csrf = session["csrf_token"]
    assert (
        run(
            "systemctl", "is-enabled", "limeos-storage-targets", check=False
        ).stdout.strip()
        == "static"
    )
    assert (
        run("systemctl", "is-active", "limeos-storage-targets", check=False).returncode
        != 0
    )
    assert not POLICY.exists()
    shadow = next(repo.rglob(f"limeos-shadow_{VERSION}_amd64.deb"))
    with tempfile.TemporaryDirectory(prefix="limeos-target-shadow-") as extracted:
        run("dpkg-deb", "-x", str(shadow), extracted)
        assert not (
            Path(extracted) / "lib/systemd/system/limeos-shadow-storage-targets.service"
        ).exists()
    parent = Path("/mnt/approved-targets")
    parent.mkdir(mode=0o755)
    run("umount", "/mnt/data2")
    device = storage.assignment(
        "data", "/dev/vdb2", str(parent / "Data"), "limeos-test-0"
    )

    def selection(name):
        contract = planning.contract(
            "single_disk", [{**device, "mountpoint": str(parent / name)}]
        )
        contract["locations"].update(
            media_host=str(parent / name / "media"),
            downloads_host=str(parent / name / "downloads"),
        )
        return contract

    allowed = [
        "Data",
        "ReviewChanged",
        "Blocked",
        "AfterCrash",
        "CeilingChanged",
        "MissingReceipt",
    ]
    policy = {
        "version": 1,
        "core_uid": int(run("id", "-u", "limeos-core").stdout),
        "allow_prepare_targets": True,
        "managed_targets": [
            {
                "filesystem_uuid": device["filesystem_uuid"],
                "mountpoint": str(parent / name),
            }
            for name in allowed
        ],
    }
    assert http(BASE + "/plans", "POST", selection("Data"), cookie, csrf)[0] == 503
    POLICY.write_text(json.dumps(policy))
    POLICY.chmod(0o644)
    run("systemctl", "start", "limeos-storage-targets")
    pid = int(
        run(
            "systemctl", "show", "limeos-storage-targets", "-p", "MainPID", "--value"
        ).stdout
    )
    status = Path(f"/proc/{pid}/status").read_text()
    assert (
        "CapEff:\t0000000000000001" in status
        and "CapBnd:\t0000000000000001" in status
        and "NoNewPrivs:\t1" in status
    )
    assert (
        "CAP_SYS_ADMIN"
        not in run(
            "systemctl",
            "show",
            "limeos-storage-targets",
            "-p",
            "CapabilityBoundingSet",
            "--value",
        ).stdout
    )
    for name in ["Unlisted", "data"]:
        assert http(BASE + "/plans", "POST", selection(name), cookie, csrf)[0] == 403
    denied = run(
        "runuser",
        "-u",
        "nobody",
        "-g",
        "limeos-host-access",
        "--",
        "python3",
        "-c",
        "import socket,struct,json;s=socket.socket(socket.AF_UNIX);s.connect('/run/limeos-storage-targets/executor.sock');b=b'{\"operation\":\"health\",\"version\":1}';s.sendall(struct.pack('!I',len(b))+b);n=struct.unpack('!I',s.recv(4))[0];print(s.recv(n).decode())",
    )
    assert json.loads(denied.stdout) == {
        "version": 1,
        "ready": False,
        "error": "forbidden",
    }
    passed(
        "dormant standard-only target service requires an explicit exact UUID/path ceiling and kernel core UID; only CAP_CHOWN is effective and no mount capability exists"
    )

    def propose(name):
        code, _, proposal = http(BASE + "/plans", "POST", selection(name), cookie, csrf)
        assert code == 201, (code, proposal)
        assert proposal["plan"]["version"] == 2
        return proposal

    def approve(proposal):
        code, _, approval = http(
            BASE + f"/plans/{proposal['plan']['id']}/approval",
            "POST",
            {"digest": proposal["digest"]},
            cookie,
            csrf,
        )
        assert code == 200, (code, approval)
        return approval["token"]

    def queue(proposal, approval, key):
        body = {"proposal": proposal, "approval": approval, "idempotency_key": key}
        code, _, job = http(BASE + "/jobs", "POST", body, cookie, csrf)
        assert code == 202, (code, job)
        return job, body

    def job_state(job):
        return locks.state(job["id"])

    fstab_before = Path("/etc/fstab").read_bytes()
    run("mount", "/dev/vdb2", "/mnt/data2")
    sentinel_before = (Path("/mnt/data2") / "sentinel").read_bytes()
    run("umount", "/mnt/data2")
    old = http(
        planning.BASE + "/plans",
        "POST",
        {
            "contract": selection("Data"),
            "inventory_digest": http(planning.BASE + "/inventory", cookie=cookie)[2][
                "digest"
            ],
            "timeout_seconds": 10,
        },
        cookie,
        csrf,
    )
    assert old[0] == 201
    preview = old[2]
    preview_nonce = http(
        planning.BASE + f"/plans/{preview['plan']['id']}/approval",
        "POST",
        {"digest": preview["digest"]},
        cookie,
        csrf,
    )[2]["token"]
    proposal = propose("Data")
    assert (
        http(
            BASE + "/jobs",
            "POST",
            {
                "proposal": proposal,
                "approval": preview_nonce,
                "idempotency_key": "old-preview",
            },
            cookie,
            csrf,
        )[0]
        == 403
    )
    assert not (parent / "Data").exists()
    for path in [
        BASE + "/plans",
        BASE + "/jobs",
        BASE + f"/plans/{proposal['plan']['id']}/approval",
    ]:
        assert http(path, "GET", cookie=cookie)[0] == 405
    assert (
        http(BASE + "/plans", "POST", selection("Data"), cookie, "bad-csrf")[0] == 403
    )
    assert (
        http(
            BASE + "/plans",
            "POST",
            selection("Data"),
            cookie,
            csrf,
            origin="https://foreign.invalid",
        )[0]
        == 403
    )
    token = approve(proposal)
    modified = copy.deepcopy(proposal)
    modified["plan"]["preparation"]["targets"][0]["parent"]["inode"] += 1
    assert (
        http(
            BASE + "/jobs",
            "POST",
            {
                "proposal": modified,
                "approval": token,
                "idempotency_key": "changed-plan",
            },
            cookie,
            csrf,
        )[0]
        == 409
    )
    job, body = queue(proposal, token, "prepare-data")
    eventually(lambda: job_state(job) == "succeeded")
    target = parent / "Data"
    assert (
        target.stat().st_uid == target.stat().st_gid == 0
        and target.stat().st_mode & 0o777 == 0o755
        and list(target.iterdir()) == []
    )
    assert locks.claims(job["id"]) == []
    with sqlite3.connect(planning.DB) as db:
        result = db.execute(
            "SELECT receipt,verification FROM storage_target_results WHERE job=?",
            (job["id"],),
        ).fetchone()
        assert (
            json.loads(result[0])["state"] == "verified"
            and json.loads(result[1])["targets"][0]["existing"]["inode"]
            == target.stat().st_ino
        )
        before = db.execute("SELECT count(*) FROM events").fetchone()[0]
    run("systemctl", "stop", "limeos-storage-targets")
    replay = http(BASE + "/jobs", "POST", body, cookie, csrf)
    assert replay[0] == 202 and replay[2]["id"] == job["id"]
    assert http(BASE + f"/jobs/{job['id']}", cookie=cookie)[2]["state"] == "succeeded"
    with sqlite3.connect(planning.DB) as db:
        assert db.execute("SELECT count(*) FROM events").fetchone()[0] == before
    run("systemctl", "start", "limeos-storage-targets")
    passed(
        "new human approval binds the complete target plan, rejects preview tokens and edits, creates only one empty root-owned directory, and commits independent inode verification before releasing core claims; read/replay remains pure with the executor stopped"
    )

    stale = propose("ReviewChanged")
    (parent / "ReviewChanged").mkdir()
    assert (
        http(
            BASE + f"/plans/{stale['plan']['id']}/approval",
            "POST",
            {"digest": stale["digest"]},
            cookie,
            csrf,
        )[0]
        == 409
    )
    assert (
        http(BASE + f"/plans/{stale['plan']['id']}/cancel", "POST", None, cookie, csrf)[
            0
        ]
        == 204
    )
    passed(
        "fresh target identity changes after review block approval and cancellation remains independent of inspection"
    )

    # Block the installed root service after its durable Prepared receipt and
    # before mkdir. All instrumentation is confined to the guarded test guest.
    blocked = propose("Blocked")
    blocked_nonce = approve(blocked)
    blkid = Path("/usr/sbin/blkid")
    original = Path("/usr/sbin/blkid.limeos-approved-original")
    marker = Path("/run/limeos-approved-probe-blocked")
    action = blocked["plan"]["preparation"]["action"]
    blkid.rename(original)
    blkid.write_text(
        "#!/usr/bin/python3\nimport os,sqlite3,time\nfrom pathlib import Path\nwith sqlite3.connect('/var/lib/limeos/executors/storage/targets.sqlite') as d:\n    prepared=d.execute(\"SELECT count(*) FROM receipts WHERE action=? AND state='prepared'\",("
        + repr(action)
        + ",)).fetchone()[0]\nif prepared:\n    Path('/run/limeos-approved-probe-blocked').touch()\n    time.sleep(10)\nos.execv('/usr/sbin/blkid.limeos-approved-original',['/usr/sbin/blkid.limeos-approved-original',*os.sys.argv[1:]])\n"
    )
    blkid.chmod(0o755)
    try:
        blocked_job, blocked_body = queue(blocked, blocked_nonce, "blocked-job")
        eventually(marker.exists)
        assert (
            job_state(blocked_job) == "running"
            and len(locks.claims(blocked_job["id"])) == 3
            and not (parent / "Blocked").exists()
        )
        # Kill both installed core and target executor during the same dispatch.
        core_pid = int(
            run("systemctl", "show", "limeos-core", "-p", "MainPID", "--value").stdout
        )
        os.kill(core_pid, signal.SIGKILL)
        run(
            "systemctl",
            "kill",
            "--signal=KILL",
            "--kill-who=all",
            "limeos-storage-targets",
        )
    finally:
        blkid.unlink()
        original.rename(blkid)
        marker.unlink(missing_ok=True)
    eventually(
        lambda: run("/usr/lib/limeos/limeosctl", "status", check=False).returncode == 0
    )
    eventually(lambda: job_state(blocked_job) == "needs_intervention")
    assert (
        len(locks.claims(blocked_job["id"])) == 3 and not (parent / "Blocked").exists()
    )
    pending = json.loads(
        run(EXECUTOR, "storage-target-receipt", "--action", action).stdout
    )
    assert pending["state"] == "prepared"
    assert (
        http(BASE + f"/jobs/{blocked_job['id']}/cancel", "POST", None, cookie, csrf)[0]
        == 409
    )
    following = propose("AfterCrash")
    following_nonce = approve(following)
    following_job, _ = queue(following, following_nonce, "following-job")
    time.sleep(4)
    assert job_state(following_job) == "queued" and not (parent / "AfterCrash").exists()
    replay = http(BASE + "/jobs", "POST", blocked_body, cookie, csrf)
    assert replay[0] == 202 and replay[2]["id"] == blocked_job["id"]
    assert not (parent / "Blocked").exists()
    passed(
        "real installed core and target-service SIGKILL after Prepared keeps all core/root claims; receipt-only recovery and lost-response replay cannot repeat mkdir, cancel uncertain work or dispatch a conflicting job"
    )
    resolved = json.loads(
        run(EXECUTOR, "storage-reconcile-targets", "--action", action).stdout
    )
    assert (
        resolved["state"] == "precondition_changed"
        and not (parent / "Blocked").exists()
    )
    eventually(lambda: job_state(blocked_job) == "precondition_changed", timeout=45)
    eventually(lambda: job_state(following_job) == "succeeded", timeout=45)
    assert locks.claims(blocked_job["id"]) == locks.claims(following_job["id"]) == []
    passed(
        "explicit root observation resolves unchanged interrupted targets without replay; the bound terminal receipt releases core claims and permits the already approved waiting job"
    )
    # A private authority fault models death after a committed core claim but
    # before IPC delivery. No effect request or root receipt exists for it.
    missing = propose("MissingReceipt")
    missing_nonce = approve(missing)
    run("systemctl", "stop", "limeos-core")
    with sqlite3.connect(planning.DB) as db:
        owner = missing["plan"]["principal"]
        accepted = db.execute(
            "SELECT approval_digest FROM storage_target_plans WHERE id=?",
            (missing["plan"]["id"],),
        ).fetchone()[0]
        assert accepted == hashlib.sha256(missing_nonce.encode()).hexdigest()
        action_missing = missing["plan"]["preparation"]["action"]
        intent = json.dumps(
            {"operation": "storage_prepare_targets", "plan": missing["plan"]},
            separators=(",", ":"),
            ensure_ascii=False,
        )
        db.execute(
            "INSERT INTO jobs VALUES(?,?,?,?,?,?,'queued',?,?,?)",
            (
                action_missing,
                owner,
                "missing-delivery",
                intent,
                hashlib.sha256(intent.encode()).hexdigest(),
                "storage:configuration",
                db.execute("SELECT generation FROM meta").fetchone()[0],
                missing["plan"]["grant_revision"],
                missing["plan"]["preparation"]["expires_at"],
            ),
        )
        for resource in [
            "storage:uuid:" + device["filesystem_uuid"],
            "storage:mount:" + str(parent / "MissingReceipt"),
        ]:
            db.execute(
                "INSERT INTO job_resources VALUES(?,?)", (action_missing, resource)
            )
        db.execute(
            "UPDATE storage_target_plans SET job=? WHERE id=?",
            (action_missing, missing["plan"]["id"]),
        )
        db.execute("UPDATE jobs SET state='running' WHERE id=?", (action_missing,))
    run("systemctl", "start", "limeos-core")
    eventually(lambda: locks.state(action_missing) == "needs_intervention")
    assert (
        len(locks.claims(action_missing)) == 3
        and not (parent / "MissingReceipt").exists()
    )
    assert (
        run(
            EXECUTOR, "storage-target-receipt", "--action", action_missing, check=False
        ).returncode
        != 0
    )
    proof = Path("/root/missing-target-plan.json")
    proof.write_text(json.dumps(missing["plan"]["preparation"]))
    proof.chmod(0o600)
    no_effect = json.loads(
        run(EXECUTOR, "storage-reconcile-targets", "--plan", str(proof)).stdout
    )
    assert (
        no_effect["state"] == "precondition_changed"
        and not (parent / "MissingReceipt").exists()
    )
    eventually(
        lambda: locks.state(action_missing) == "precondition_changed", timeout=45
    )
    assert locks.claims(action_missing) == []
    passed(
        "private pre-delivery core fault retains complete claims despite a missing root receipt; explicit protected-plan reconciliation independently proves unchanged targets and releases the bound job without mkdir"
    )
    assert Path("/etc/fstab").read_bytes() == fstab_before
    assert (
        not (parent / "Data/media").exists()
        and not (parent / "Data/downloads").exists()
    )
    run("mount", "/dev/vdb2", "/mnt/data2")
    assert (Path("/mnt/data2") / "sentinel").read_bytes() == sentinel_before
    passed(
        "approved target jobs and interruption preserve fstab, mounted fixture data and all media/download path leaves"
    )
    passed(package_tests.check(repo, VERSION))
    assert http(BASE + f"/jobs/{job['id']}", cookie=cookie)[2]["state"] == "succeeded"
    evidence.update(
        scope="installed approved empty target jobs with complete core claims and receipt-only recovery; mount/fstab effects remain pending",
        authority_schema=8,
        package_version=VERSION,
        upgrade=upgrade,
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
    )
    evidence["limitations"] = [
        "Disposable AMD64 VM; no new native ARM64/Pi qualification",
        "Only empty target directory preparation is executable; mount/fstab, live dependency discovery, unmount/loss handling and guided UI remain pending",
        "Root operator journal and core job authority retain independent barriers; root reconciliation remains explicit",
    ]
    output.write_text(json.dumps(evidence, indent=2) + "\n")


if __name__ == "__main__":
    main()
