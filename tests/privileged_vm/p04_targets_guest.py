#!/usr/bin/env python3
"""Protected target preparation and interruption recovery on guarded VM disks."""

import copy
import hashlib
import json
import os
import shutil
import signal
import sqlite3
import subprocess
import sys
import time
from pathlib import Path

import p04_planning_guest as planning
import p04_storage_guest as readiness

VERSION = "0.4.2"
EXECUTOR = readiness.EXECUTOR
STATE = Path("/var/lib/limeos/executors/storage")
run = readiness.run


def main():
    if (
        os.geteuid() != 0
        or Path("/etc/hostname").read_text().strip() != "limeos-p01-test"
    ):
        raise SystemExit("Refusing outside disposable VM")
    planning.VERSION = VERSION
    planning.main()
    output = Path(sys.argv[2])
    evidence = json.loads(output.read_text())
    checks = evidence["passed"]

    def passed(name):
        checks.append(name)
        print("PASS " + name, flush=True)

    assert STATE.stat().st_uid == 0 and STATE.stat().st_mode & 0o777 == 0o700
    fstab_before = Path("/etc/fstab").read_bytes()
    sentinels = {
        name: (Path("/mnt") / name / "sentinel").read_bytes()
        for name in ["data", "data2", "downloads", "parity", "backup"]
    }
    data = readiness.assignment("data", "/dev/vdb1", "/mnt/data", "limeos-test-0")
    second = readiness.assignment("data2", "/dev/vdb2", "/mnt/data2", "limeos-test-0")
    parent = Path("/mnt/preparation")
    parent.mkdir(mode=0o755)
    contract_file = Path("/root/target-contract.json")
    plan_file = Path("/root/target-plan.json")

    def selection(target="/mnt/preparation/Data"):
        value = planning.contract("single_disk", [{**second, "mountpoint": target}])
        value["locations"].update(
            media_host=target + "/media", downloads_host=target + "/downloads"
        )
        return value

    def make_plan(contract=None, check=True):
        contract_file.write_text(json.dumps(contract or selection()))
        contract_file.chmod(0o600)
        result = run(
            EXECUTOR,
            "storage-target-plan",
            "--contract",
            str(contract_file),
            check=check,
        )
        return json.loads(result.stdout) if result.returncode == 0 else result

    def prepare(plan, check=True):
        plan_file.write_text(json.dumps(plan))
        plan_file.chmod(0o600)
        return run(
            EXECUTOR, "storage-prepare-targets", "--plan", str(plan_file), check=check
        )

    def inventory():
        return json.loads(run(EXECUTOR, "storage-inventory").stdout)

    # A swap file on the first partition excludes both partitions' shared disk.
    before_swap = inventory()
    run(
        "systemctl",
        "start",
        "limeos-storaged",
        "limeos-containerd",
        "limeos-core",
        "limeos-storage-reader",
    )
    for _ in range(30):
        if run("/usr/lib/limeos/limeosctl", "status", check=False).returncode == 0:
            break
        time.sleep(0.1)
    with sqlite3.connect(planning.DB) as db:
        db.execute(
            "INSERT INTO grants VALUES(?,?,?)",
            ("legacy-0", '"storage_manage"', "storage:configuration"),
        )
    passwords = json.loads(Path("/root/werkzeug-hashes.json").read_text())
    code, headers, session = planning.http(
        "/api/v1/auth/login",
        "POST",
        {"username": "legacy-0", "password": passwords[0]["password"]},
    )
    assert code == 200
    cookie, csrf = headers["set-cookie"].split(";", 1)[0], session["csrf_token"]
    code, _, view = planning.http(planning.BASE + "/inventory", cookie=cookie)
    assert code == 200 and view["inventory"] == before_swap
    selected = planning.contract("single_disk", [data])
    code, _, swap_preview = planning.http(
        planning.BASE + "/plans",
        "POST",
        {
            "contract": selected,
            "inventory_digest": view["digest"],
            "timeout_seconds": 10,
        },
        cookie,
        csrf,
    )
    assert code == 201
    swap = Path("/mnt/data/fixture.swap")
    run("dd", "if=/dev/zero", "of=" + str(swap), "bs=1M", "count=16", "status=none")
    swap.chmod(0o600)
    run("mkswap", str(swap))
    try:
        run("swapon", str(swap))
        active = inventory()
        affected = [
            d
            for d in active["devices"]
            if d["filesystem_uuid"]
            in [data["filesystem_uuid"], second["filesystem_uuid"]]
        ]
        assert len(affected) == 2 and all(d["in_use_as_swap"] for d in affected)
        assert active["topology_digest"] != before_swap["topology_digest"]
        readiness.check_error([data], "active_swap")
        assert make_plan(check=False).returncode != 0
        assert (
            planning.http(
                planning.BASE + "/plans/" + swap_preview["plan"]["id"] + "/approval",
                "POST",
                {"digest": swap_preview["digest"]},
                cookie,
                csrf,
            )[0]
            == 409
        )
        code, _, active_view = planning.http(
            planning.BASE + "/inventory", cookie=cookie
        )
        assert code == 200 and active_view["inventory"] == active
        assert (
            planning.http(
                planning.BASE + "/plans",
                "POST",
                {
                    "contract": selected,
                    "inventory_digest": active_view["digest"],
                    "timeout_seconds": 10,
                },
                cookie,
                csrf,
            )[0]
            == 409
        )
    finally:
        run("swapoff", str(swap), check=False)
        swap.unlink()
    assert inventory() == before_swap
    passed(
        "fresh active swap file identity excludes its backing disk and sibling partition; HTTP creation/stale approval refuse it and swapoff restores the fingerprint"
    )

    assert make_plan(check=False).returncode != 0  # Selected device still mounted.
    run("umount", "/mnt/data2")
    plan = make_plan()
    assert (
        plan["operation"] == "storage.prepare_targets"
        and plan["targets"][0]["existing"] is None
    )
    assert not Path(plan["targets"][0]["mountpoint"]).exists()
    receipt = json.loads(prepare(plan).stdout)
    relocated = Path("/root/relocated-executor")
    shutil.copyfile(EXECUTOR, relocated)
    relocated.chmod(0o755)
    assert (
        run(
            str(relocated),
            "storage-prepare-targets",
            "--plan",
            str(plan_file),
            check=False,
        ).returncode
        != 0
    )
    assert (
        run(
            str(relocated),
            "storage-reconcile-targets",
            "--action",
            plan["action"],
            check=False,
        ).returncode
        != 0
    )
    relocated.unlink()
    target = Path(plan["targets"][0]["mountpoint"])
    assert (
        receipt["state"] == "verified"
        and target.stat().st_uid == target.stat().st_gid == 0
    )
    assert target.stat().st_mode & 0o777 == 0o755 and list(target.iterdir()) == []
    with sqlite3.connect(STATE / "targets.sqlite") as db:
        assert db.execute("PRAGMA user_version").fetchone()[0] == 1
        assert db.execute("SELECT count(*) FROM locks").fetchone()[0] == 0
        journal_before = db.execute("SELECT * FROM receipts").fetchall()
    assert json.loads(prepare(plan).stdout) == receipt
    assert (
        json.loads(
            run(EXECUTOR, "storage-target-receipt", "--action", plan["action"]).stdout
        )
        == receipt
    )
    with sqlite3.connect(STATE / "targets.sqlite") as db:
        assert db.execute("SELECT * FROM receipts").fetchall() == journal_before
    existing = make_plan()
    assert existing["targets"][0]["existing"] is not None
    assert json.loads(prepare(existing).stdout)["state"] == "verified"
    passed(
        "installed root operator preparation creates only an empty 0755 target; verified receipts survive lookup and idempotent replay"
    )

    unsafe = parent / "unsafe"
    for kind in ["symlink", "file", "nonempty", "writable", "foreign_owner"]:
        if kind == "symlink":
            unsafe.symlink_to(target)
        elif kind == "file":
            unsafe.write_text("preserve")
        else:
            unsafe.mkdir(mode=0o755)
            if kind == "nonempty":
                (unsafe / "preserve").write_text("preserve")
            elif kind == "writable":
                unsafe.chmod(0o777)
            else:
                os.chown(unsafe, 1000, 1000)
        assert make_plan(selection(str(unsafe)), check=False).returncode != 0, kind
        if unsafe.is_symlink() or unsafe.is_file():
            unsafe.unlink()
        else:
            if kind == "nonempty":
                assert (unsafe / "preserve").read_text() == "preserve"
                (unsafe / "preserve").unlink()
            unsafe.rmdir()
    parent.chmod(0o777)
    assert make_plan(check=False).returncode != 0
    parent.chmod(0o755)
    unsafe.symlink_to(parent)
    assert make_plan(selection(str(unsafe) + "/child"), check=False).returncode != 0
    unsafe.unlink()
    assert (
        make_plan(selection("/mnt/missing-parent/child"), check=False).returncode != 0
    )
    assert make_plan(selection("/mnt/data/child"), check=False).returncode != 0
    passed(
        "symlinks, writable or foreign-owned paths, bare data, absent parents and targets beneath another mount are refused without alteration"
    )

    stale = make_plan(selection(str(parent / "stale")))
    (parent / "stale").mkdir()
    assert prepare(stale, check=False).returncode != 0
    (parent / "stale").rmdir()
    for field, value in [
        ("version", 2),
        ("operation", "storage.mount"),
        ("expires_at", stale["created_at"]),
        ("command", "mount -a"),
    ]:
        invalid = copy.deepcopy(stale)
        invalid[field] = value
        assert prepare(invalid, check=False).returncode != 0
    stale_fstab = make_plan(selection(str(parent / "stale-fstab")))
    Path("/etc/fstab").write_bytes(fstab_before + b"# changed during target review\n")
    assert prepare(stale_fstab, check=False).returncode != 0
    Path("/etc/fstab").write_bytes(fstab_before)
    with sqlite3.connect(planning.DB) as db:
        preview = json.loads(
            db.execute("SELECT body FROM storage_plans LIMIT 1").fetchone()[0]
        )
    assert prepare(preview, check=False).returncode == 2
    plan_file.chmod(0o666)
    assert (
        run(
            EXECUTOR, "storage-prepare-targets", "--plan", str(plan_file), check=False
        ).returncode
        != 0
    )
    with sqlite3.connect(STATE / "targets.sqlite") as db:
        assert db.execute("SELECT count(*) FROM receipts").fetchone()[0] == 2
    passed(
        "changed target/fstab, expiry, free-form requests and old API previews cannot authorize target preparation or create receipts"
    )

    # Block real raw probes after the durable receipt, then kill the executor.
    # The wrapper and coordination files live only inside this guarded guest.
    interrupted = make_plan(selection(str(parent / "interrupted")))
    plan_file.write_text(json.dumps(interrupted))
    plan_file.chmod(0o600)
    blkid = Path("/usr/sbin/blkid")
    original_blkid = Path("/usr/sbin/blkid.limeos-fixture-original")
    blkid.rename(original_blkid)
    blocker = Path("/run/limeos-target-probe-blocked")
    blkid.write_text(
        "#!/usr/bin/python3\nimport os, sqlite3, time\nfrom pathlib import Path\np=Path('/var/lib/limeos/executors/storage/targets.sqlite')\nwith sqlite3.connect(p) as d:\n    prepared=d.execute(\"SELECT count(*) FROM receipts WHERE state='prepared'\").fetchone()[0]\nif prepared:\n    Path('/run/limeos-target-probe-blocked').touch()\n    time.sleep(10)\nos.execv('/usr/sbin/blkid.limeos-fixture-original', ['/usr/sbin/blkid.limeos-fixture-original', *os.sys.argv[1:]])\n"
    )
    blkid.chmod(0o755)
    process = None
    try:
        process = subprocess.Popen(
            [EXECUTOR, "storage-prepare-targets", "--plan", str(plan_file)],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            start_new_session=True,
        )
        deadline = time.monotonic() + 10
        while (
            not blocker.exists()
            and time.monotonic() < deadline
            and process.poll() is None
        ):
            time.sleep(0.02)
        assert blocker.exists(), process.communicate(timeout=2)
        os.killpg(process.pid, signal.SIGKILL)
        process.communicate(timeout=5)
    finally:
        if process is not None and process.poll() is None:
            os.killpg(process.pid, signal.SIGKILL)
            process.communicate(timeout=5)
        blkid.unlink()
        original_blkid.rename(blkid)
        blocker.unlink(missing_ok=True)
    assert not (parent / "interrupted").exists()
    pending = json.loads(
        run(
            EXECUTOR, "storage-target-receipt", "--action", interrupted["action"]
        ).stdout
    )
    assert pending["state"] == "prepared"
    replay = prepare(interrupted, check=False)
    assert replay.returncode == 1 and json.loads(replay.stdout) == pending
    following = make_plan(selection(str(parent / "following")))
    assert (
        prepare(following, check=False).returncode != 0
        and not (parent / "following").exists()
    )
    with sqlite3.connect(STATE / "targets.sqlite") as db:
        assert (
            db.execute(
                "SELECT count(*) FROM locks WHERE action=?", (interrupted["action"],)
            ).fetchone()[0]
            == 3
        )
        assert db.execute("SELECT count(*) FROM receipts").fetchone()[0] == 3
    passed(
        "real executor death after durable dispatch preserves three resource claims; receipt lookup and retry cannot replay mkdir or bypass uncertainty"
    )
    resolved = json.loads(
        run(
            EXECUTOR, "storage-reconcile-targets", "--action", interrupted["action"]
        ).stdout
    )
    assert (
        resolved["state"] == "precondition_changed"
        and not (parent / "interrupted").exists()
    )
    with sqlite3.connect(STATE / "targets.sqlite") as db:
        assert db.execute("SELECT count(*) FROM locks").fetchone()[0] == 0
    passed(
        "explicit fresh reconciliation proves an unchanged target and releases its claims without replaying the effect"
    )

    interrupted_after = make_plan(selection(str(parent / "interrupted-after")))
    plan_file.write_text(json.dumps(interrupted_after))
    plan_file.chmod(0o600)
    blkid.rename(original_blkid)
    blkid.write_text(
        "#!/usr/bin/python3\nimport os, sqlite3, time\nfrom pathlib import Path\np=Path('/var/lib/limeos/executors/storage/targets.sqlite')\nwith sqlite3.connect(p) as d:\n    prepared=d.execute(\"SELECT count(*) FROM receipts WHERE state='prepared'\").fetchone()[0]\nif prepared and Path('/mnt/preparation/interrupted-after').exists():\n    Path('/run/limeos-target-probe-blocked').touch()\n    time.sleep(10)\nos.execv('/usr/sbin/blkid.limeos-fixture-original', ['/usr/sbin/blkid.limeos-fixture-original', *os.sys.argv[1:]])\n"
    )
    blkid.chmod(0o755)
    process = None
    try:
        process = subprocess.Popen(
            [EXECUTOR, "storage-prepare-targets", "--plan", str(plan_file)],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            start_new_session=True,
        )
        deadline = time.monotonic() + 10
        while (
            not blocker.exists()
            and time.monotonic() < deadline
            and process.poll() is None
        ):
            time.sleep(0.02)
        assert blocker.exists(), process.communicate(timeout=2)
        os.killpg(process.pid, signal.SIGKILL)
        process.communicate(timeout=5)
    finally:
        if process is not None and process.poll() is None:
            os.killpg(process.pid, signal.SIGKILL)
            process.communicate(timeout=5)
        blkid.unlink()
        original_blkid.rename(blkid)
        blocker.unlink(missing_ok=True)
    changed = parent / "interrupted-after"
    assert changed.exists() and list(changed.iterdir()) == []
    replay = prepare(interrupted_after, check=False)
    assert replay.returncode == 1 and json.loads(replay.stdout)["state"] == "prepared"
    # Unexpected data must prevent reconciliation and keep the claims.
    (changed / "external-data").write_text("preserve")
    assert (
        run(
            EXECUTOR,
            "storage-reconcile-targets",
            "--action",
            interrupted_after["action"],
            check=False,
        ).returncode
        != 0
    )
    with sqlite3.connect(STATE / "targets.sqlite") as db:
        assert db.execute("SELECT count(*) FROM locks").fetchone()[0] == 3
    assert (changed / "external-data").read_text() == "preserve"
    (
        changed / "external-data"
    ).unlink()  # Fixture cleanup only, never executor cleanup.
    resolved_after = json.loads(
        run(
            EXECUTOR,
            "storage-reconcile-targets",
            "--action",
            interrupted_after["action"],
        ).stdout
    )
    assert resolved_after["state"] == "verified" and list(changed.iterdir()) == []
    with sqlite3.connect(STATE / "targets.sqlite") as db:
        assert db.execute("SELECT count(*) FROM locks").fetchone()[0] == 0
    passed(
        "real executor death after mkdir keeps claims; unexpected data blocks reconciliation and fresh verified postconditions resolve the receipt without another mkdir"
    )

    for name, expected in sentinels.items():
        if name == "data2":
            run("mount", "/dev/vdb2", "/mnt/data2")
        assert (Path("/mnt") / name / "sentinel").read_bytes() == expected
    assert Path("/etc/fstab").read_bytes() == fstab_before
    assert not Path("/mnt/storage").exists() and not (target / "media").exists()
    passed(
        "all synthetic filesystem data, original fstab and media paths survive target preparation and interruption"
    )
    evidence.update(
        scope="installed protected target preparation only; mount/fstab effects and core job integration pending",
        target_payload_sha256=hashlib.sha256(Path(EXECUTOR).read_bytes()).hexdigest(),
    )
    evidence["limitations"] = [
        s for s in evidence["limitations"] if "safe target preparation" not in s
    ]
    evidence["limitations"].extend(
        [
            "Root operator CLI only; HTTP preview approvals remain non-executable",
            "Prepared/unknown target receipts require explicit fresh root reconciliation; no automatic unlock",
            "Mount/fstab effects, dependency shutdown and guided UI pending",
        ]
    )
    output.write_text(json.dumps(evidence, indent=2) + "\n")


if __name__ == "__main__":
    main()
