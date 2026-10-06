#!/usr/bin/env python3
"""Core resource claims and genuine schema-6 upgrade; disposable Debian only."""

import hashlib
import json
import os
import signal
import sqlite3
import sys
import tarfile
import time
from pathlib import Path

import p03_guest as container_tests
import p04_planning_guest as planning
import p04_targets_guest as targets

VERSION = "0.4.3"
CORE = Path("/usr/lib/limeos/limeos-core")
DB = planning.DB
run = planning.run
eventually = container_tests.eventually


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def repository(path):
    run(
        "install",
        "-m",
        "0644",
        str(path / "limeos-archive-keyring.gpg"),
        "/usr/share/keyrings/limeos-test.gpg",
    )
    Path("/etc/apt/sources.list.d/limeos-test.list").write_text(
        f"deb [signed-by=/usr/share/keyrings/limeos-test.gpg] file:{path} stable main\n"
    )
    run("apt-get", "update", "-qq", timeout=120)


def state(job):
    with sqlite3.connect(DB) as db:
        return db.execute("SELECT state FROM jobs WHERE id=?", (job,)).fetchone()[0]


def claims(job):
    with sqlite3.connect(DB) as db:
        return [
            row[0]
            for row in db.execute(
                "SELECT resource FROM resource_locks WHERE job=? ORDER BY resource",
                (job,),
            )
        ]


def fixture_job(db, owner, key, initial="queued", resources=()):
    # Extra dependencies model operation-owned sets inside the private writer;
    # closed HealthProbe fixtures have no executor effects or API claim input.
    job = hashlib.sha256(key.encode()).hexdigest()
    resource = "fixture:" + key
    intent = json.dumps({"operation": "health_probe", "resource": resource})
    generation = db.execute("SELECT generation FROM meta").fetchone()[0]
    revision = db.execute(
        "SELECT grant_revision FROM users WHERE id=?", (owner,)
    ).fetchone()[0]
    schema = db.execute("PRAGMA user_version").fetchone()[0]
    db.execute(
        "INSERT INTO jobs VALUES(?,?,?,?,?,?,?,?,?,?)",
        (
            job,
            owner,
            key,
            intent,
            hashlib.sha256(intent.encode()).hexdigest(),
            resource,
            "queued" if schema >= 7 else initial,
            generation,
            revision,
            int(time.time()) + 300,
        ),
    )
    for dependency in resources:
        db.execute("INSERT INTO job_resources VALUES(?,?)", (job, dependency))
    if schema >= 7 and initial != "queued":
        db.execute("UPDATE jobs SET state=? WHERE id=?", (initial, job))
    return job


def upgrade(
    candidate,
    previous_version="0.4.2",
    previous_schema=6,
    authority_schema=7,
    previous_package_sha256="468b844cebb12806661a4d757b47b53f0c9d2f3bcbe21bb2d89639b5df656263",
    previous_core_sha256="f868c45731f2b9323843be43b55de638648e1a228e92f29417dfad9ac42a69a9",
):
    previous = Path("/opt/limeos-previous-repo")
    if not previous.is_dir():
        return None
    architecture = run("dpkg", "--print-architecture").stdout.strip()
    old_package = next(previous.rglob(f"limeos_{previous_version}_{architecture}.deb"))
    assert digest(old_package) == previous_package_sha256, (
        "Genuinely frozen previous package required"
    )
    repository(previous)
    run("apt-get", "install", "-y", "limeos=" + previous_version, timeout=120)
    old_core = digest(CORE)
    assert old_core == previous_core_sha256
    run("apt-get", "install", "-y", "docker.io", "busybox-static", timeout=120)
    run("systemctl", "start", "docker")
    cookie, csrf, owner = container_tests.enroll()
    rootfs = Path("/root/claims-upgrade-rootfs.tar")
    with tarfile.open(rootfs, "w") as archive:
        archive.add("/bin/busybox", arcname="bin/busybox")
    run("docker", "import", str(rootfs), "limeos-claims-upgrade:local")
    identifier = run(
        "docker",
        "run",
        "-d",
        "limeos-claims-upgrade:local",
        "/bin/busybox",
        "sleep",
        "3600",
    ).stdout.strip()
    policy_path = Path("/etc/limeos/system-policy/container.json")
    policy = json.loads(policy_path.read_text())
    policy.update(allow_restart=True, managed_containers=[identifier])
    policy_path.write_text(json.dumps(policy))
    container_tests.restart_container()
    proposal, approval = container_tests.plan(identifier, cookie, csrf)
    old_job, old_body = container_tests.queue(
        proposal, approval, "upgrade-verified", cookie, csrf
    )
    eventually(lambda: state(old_job["id"]) == "succeeded")
    pending, token = container_tests.plan(identifier, cookie, csrf)
    run("systemctl", "stop", "limeos-core")
    with sqlite3.connect(DB) as db:
        assert db.execute("PRAGMA user_version").fetchone()[0] == previous_schema
        before = db.execute(
            "SELECT body,digest,approval_digest FROM container_plans WHERE id=?",
            (pending["plan"]["id"],),
        ).fetchone()
        proof = db.execute(
            "SELECT receipt,verification FROM container_results WHERE job=?",
            (old_job["id"],),
        ).fetchone()
        legacy = {
            status: fixture_job(
                db,
                owner["id"],
                "legacy-" + status,
                status,
                ["storage:uuid:legacy-" + status, "storage:mount:/mnt/Legacy-" + status]
                if previous_schema >= 7
                else [],
            )
            for status in [
                "running",
                "verifying",
                "outcome_unknown",
                "needs_intervention",
                "queued",
            ]
        }
    repository(candidate)
    run("apt-get", "install", "-y", "limeos=" + VERSION, timeout=120)
    assert digest(CORE) != old_core
    assert container_tests.http("/api/v1/overview", cookie=cookie)[0] == 200
    with sqlite3.connect(DB) as db:
        assert db.execute("PRAGMA user_version").fetchone()[0] == authority_schema
        assert (
            db.execute(
                "SELECT body,digest,approval_digest FROM container_plans WHERE id=?",
                (pending["plan"]["id"],),
            ).fetchone()
            == before
        )
        assert (
            db.execute(
                "SELECT receipt,verification FROM container_results WHERE job=?",
                (old_job["id"],),
            ).fetchone()
            == proof
        )
    for status, job in legacy.items():
        assert state(job) == (
            "needs_intervention" if status in ["running", "verifying"] else status
        )
        assert len(claims(job)) == (
            0 if status == "queued" else 3 if previous_schema >= 7 else 1
        )
    new_job, _ = container_tests.queue(pending, token, "upgrade-pending", cookie, csrf)
    eventually(lambda: state(new_job["id"]) == "succeeded")
    code, _, replay = container_tests.http(
        container_tests.BASE + "/jobs", "POST", old_body, cookie, csrf
    )
    assert code == 202 and replay["id"] == old_job["id"]
    assert claims(old_job["id"]) == [] and claims(new_job["id"]) == []
    policy.update(allow_restart=False, managed_containers=[])
    policy_path.write_text(json.dumps(policy))
    container_tests.restart_container()
    run("docker", "rm", "-f", identifier)
    return {
        "from": previous_version,
        "to": VERSION,
        "previous_package_sha256": digest(old_package),
        "previous_core_sha256": old_core,
        "current_core_sha256": digest(CORE),
        "preserved": [
            "human session",
            "approved canonical bytes and token hash",
            "verified receipt",
            "four active complete claim sets"
            if previous_schema >= 7
            else "four active primary locks",
            "queued state",
        ],
        "post_upgrade_approved_restart": "verified",
        "original_queue_response_replay": "same job",
    }


def main(package_version="0.4.3", authority_schema=7, qualify_upgrade=True):
    global VERSION
    VERSION = package_version
    if (
        os.geteuid() != 0
        or Path("/etc/hostname").read_text().strip() != "limeos-p01-test"
    ):
        raise SystemExit("Refusing outside disposable VM")
    candidate, output = map(Path, sys.argv[1:])
    upgrade_evidence = upgrade(candidate) if qualify_upgrade else None
    if upgrade_evidence:
        print(
            "PASS frozen-package upgrade and preserved approvals/receipts/legacy locks",
            flush=True,
        )
    targets.VERSION = VERSION
    planning.AUTHORITY_SCHEMA = authority_schema
    targets.main()
    evidence = json.loads(output.read_text())

    def passed(name):
        evidence["passed"].append(name)
        print("PASS " + name, flush=True)

    if upgrade_evidence:
        passed(
            "genuine frozen 0.4.2 to 0.4.3 upgrade preserves sessions, pending approval bytes, verified receipts and every legacy active lock; original replay and newly verified approved restart pass"
        )
    with sqlite3.connect(DB) as db:
        owner = db.execute("SELECT id FROM users WHERE username='legacy-0'").fetchone()[
            0
        ]
        a = fixture_job(
            db,
            owner,
            "claim-a",
            resources=[
                "storage:configuration",
                "storage:uuid:fixture-a",
                "storage:mount:/mnt/Claims",
            ],
        )
        b = fixture_job(
            db,
            owner,
            "claim-b",
            resources=["storage:configuration", "storage:uuid:fixture-b"],
        )
        independent = fixture_job(
            db, owner, "claim-independent", resources=["storage:uuid:independent"]
        )
        db.execute("UPDATE jobs SET state='running' WHERE id=?", (a,))
        try:
            db.execute("UPDATE jobs SET state='running' WHERE id=?", (b,))
        except sqlite3.IntegrityError:
            pass
        else:
            raise AssertionError("Conflicting dispatch accepted")
        assert (
            db.execute("SELECT state FROM jobs WHERE id=?", (b,)).fetchone()[0]
            == "queued"
        )
        assert (
            db.execute(
                "SELECT count(*) FROM resource_locks WHERE job=?", (b,)
            ).fetchone()[0]
            == 0
        )
        db.execute("UPDATE jobs SET state='running' WHERE id=?", (independent,))
    held = claims(a)
    assert len(held) == 4 and len(claims(independent)) == 2
    passed(
        "installed schema acquires complete claim sets atomically; overlap has no partial locks and an unrelated job can dispatch"
    )
    with sqlite3.connect(DB) as db:
        for statement in [
            "DELETE FROM resource_locks WHERE job=?",
            "DELETE FROM job_resources WHERE job=?",
            "UPDATE jobs SET deadline=9999999999 WHERE id=?",
        ]:
            try:
                db.execute(statement, (a,))
            except sqlite3.IntegrityError:
                pass
            else:
                raise AssertionError("Active claims changed")
    assert claims(a) == held
    passed(
        "installed active claims, required sets and job identity reject deletion or expiry extension"
    )
    old_pid = int(
        run("systemctl", "show", "limeos-core", "-p", "MainPID", "--value").stdout
    )
    os.kill(old_pid, signal.SIGKILL)
    eventually(
        lambda: (
            state(a) == "needs_intervention"
            and state(independent) == "needs_intervention"
        )
    )
    eventually(
        lambda: run("/usr/lib/limeos/limeosctl", "status", check=False).returncode == 0
    )
    assert claims(a) == held and len(claims(independent)) == 2 and state(b) == "queued"
    passed(
        "real installed core SIGKILL and automatic restart retain every claim and recover running work without effect replay"
    )
    run("systemctl", "stop", "limeos-core")
    with sqlite3.connect(DB) as db:
        original_guard = db.execute(
            "SELECT sql FROM sqlite_schema WHERE name='resource_locks_delete'"
        ).fetchone()[0]
        generation = db.execute("SELECT generation FROM meta").fetchone()[0]
        audit = db.execute("SELECT count(*) FROM events").fetchone()[0]
        db.execute("DROP TRIGGER resource_locks_delete")
        db.execute(
            "DELETE FROM resource_locks WHERE job=? AND resource='storage:configuration'",
            (a,),
        )
        db.execute(original_guard)
    refusal = run("runuser", "-u", "limeos-core", "--", str(CORE), check=False)
    assert refusal.returncode == 1 and "StateNotDurable" in refusal.stdout
    with sqlite3.connect(DB) as db:
        assert db.execute("SELECT generation FROM meta").fetchone()[0] == generation
        assert db.execute("SELECT count(*) FROM events").fetchone()[0] == audit
        db.execute("INSERT INTO resource_locks VALUES('storage:configuration',?)", (a,))
    passed(
        "installed startup refuses missing dependency locks without advancing generation, writing recovery events or automatically rebuilding claims"
    )
    with sqlite3.connect(DB) as db:
        guard = db.execute(
            "SELECT sql FROM sqlite_schema WHERE name='jobs_acquire_resources'"
        ).fetchone()[0]
        db.execute("DROP TRIGGER jobs_acquire_resources")
        db.execute(
            "CREATE TRIGGER jobs_acquire_resources AFTER UPDATE OF state ON jobs BEGIN SELECT 1; END"
        )
    refusal = run("runuser", "-u", "limeos-core", "--", str(CORE), check=False)
    assert refusal.returncode == 1 and "StateNotDurable" in refusal.stdout
    with sqlite3.connect(DB) as db:
        assert db.execute("SELECT generation FROM meta").fetchone()[0] == generation
        assert db.execute("SELECT count(*) FROM events").fetchone()[0] == audit
        db.execute("DROP TRIGGER jobs_acquire_resources")
        db.execute(guard)
    passed(
        "installed startup rejects altered dispatch guards even when SQLite quick_check and every existing claim are valid"
    )
    run("systemctl", "start", "limeos-core")
    eventually(
        lambda: run("/usr/lib/limeos/limeosctl", "status", check=False).returncode == 0
    )
    assert claims(a) == held
    evidence.update(
        scope="installed core multi-resource lock foundation, storage target regressions and optional genuine frozen-package upgrade",
        authority_schema=authority_schema,
        core_payload_sha256=digest(CORE),
        upgrade=upgrade_evidence,
    )
    evidence["limitations"].extend(
        [
            "Extra claim sets are private synthetic HealthProbe fixtures; no caller-supplied dependency list or storage execution API exists",
            "Container mount dependency discovery and shared executor dispatch remain pending; the root target journal is still separate",
        ]
    )
    output.write_text(json.dumps(evidence, indent=2) + "\n")


if __name__ == "__main__":
    main()
