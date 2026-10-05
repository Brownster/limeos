#!/usr/bin/env python3
"""Fresh storage identity and bounded readiness; disposable Debian guest only."""

import hashlib
import http.client
import json
import os
import sqlite3
import subprocess
import sys
import time
from pathlib import Path

EXECUTOR = "/usr/lib/limeos/limeos-executor"
PLAN = Path("/etc/limeos/storage-wait.json")
VERSION = "0.4.0"


def run(*args, input=None, check=True, timeout=30):
    value = subprocess.run(
        args, input=input, text=True, capture_output=True, timeout=timeout, check=False
    )
    if check and value.returncode:
        if args == ("systemctl", "start", "limeos-storage-ready"):
            diagnostic = """import os, subprocess
for p in ['/proc/self/ns/mnt', '/proc/1/ns/mnt']:
    try: print(p, os.stat(p).st_ino)
    except OSError as e: print(p, e)
for p in ['/proc/self/mountinfo', '/proc/1/mountinfo']:
    try:
        print(p, next(l.split()[0] for l in open(p) if l.split()[4] == '/'))
    except OSError as e: print(p, e)
fd = os.open('/dev/vdb1', os.O_RDONLY)
r = subprocess.run(['/usr/sbin/blkid', '-p', '-o', 'export', '-s', 'UUID', '-s', 'TYPE', f'/proc/{os.getpid()}/fd/{fd}'], capture_output=True, text=True)
print('raw-probe', r.returncode, r.stdout, r.stderr)
"""
            context = subprocess.run(
                [
                    "systemd-run",
                    "--wait",
                    "--collect",
                    "--pipe",
                    "--property",
                    "CapabilityBoundingSet=",
                    "--property",
                    "NoNewPrivileges=yes",
                    "/usr/bin/python3",
                    "-c",
                    diagnostic,
                ],
                capture_output=True,
                text=True,
                check=False,
                timeout=15,
            )
            print("READINESS CONTEXT: " + context.stdout + context.stderr, flush=True)
        raise AssertionError(f"{args[0]} failed: {value.stderr[-2000:]}")
    return value


def write_plan(devices, timeout=3):
    PLAN.write_text(json.dumps({"timeout_seconds": timeout, "devices": devices}))
    PLAN.chmod(0o600)


def uuid(device):
    return run("blkid", "-p", "-s", "UUID", "-o", "value", device).stdout.strip()


def assignment(identifier, device, mount, serial=None, role="data", filesystem="ext4"):
    result = {
        "id": identifier,
        "role": role,
        "filesystem_uuid": uuid(device),
        "filesystem": filesystem,
        "mountpoint": mount,
    }
    if serial:
        result["serial"] = serial
    return result


def check_error(devices, error, *, path=PLAN, command="storage-check"):
    write_plan(devices)
    before = hashlib.sha256(PLAN.read_bytes()).hexdigest()
    value = run(EXECUTOR, command, "--plan", str(path), check=False)
    assert value.returncode != 0, value.stdout
    assert json.loads(value.stderr)["error"] == error, value.stderr
    assert hashlib.sha256(PLAN.read_bytes()).hexdigest() == before
    return value


def http_login(username, password):
    connection = http.client.HTTPConnection("127.0.0.1", 8003, timeout=12)
    connection.request(
        "POST",
        "/api/v1/auth/login",
        json.dumps({"username": username, "password": password}),
        {"Content-Type": "application/json", "Origin": "https://localhost"},
    )
    response = connection.getresponse()
    response.read()
    code = response.status
    connection.close()
    return code


def main():
    if (
        os.geteuid() != 0
        or Path("/etc/hostname").read_text().strip() != "limeos-p01-test"
    ):
        raise SystemExit("Refusing outside disposable VM")
    # Validate every extra disk before any partition/format command. The runner
    # created empty qcow2 files with these serials; no host disks are attached.
    for index, name in enumerate(["vdb", "vdc", "vdd", "vde"]):
        assert (
            Path(f"/sys/class/block/{name}/serial").read_text().strip()
            == f"limeos-test-{index}"
        )
        assert (
            int(run("blockdev", "--getsize64", f"/dev/{name}").stdout)
            == 128 * 1024 * 1024
        )
        assert not run(
            "lsblk", "-n", "-o", "MOUNTPOINTS", f"/dev/{name}"
        ).stdout.strip()
    repo, output = map(Path, sys.argv[1:])
    checks = []

    def passed(name):
        checks.append(name)
        print("PASS " + name, flush=True)

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
        f"limeos={VERSION}",
        "fdisk",
        "dmsetup",
        "e2fsprogs",
        timeout=120,
    )
    run("dpkg", "--verify", "limeos")
    run("systemd-analyze", "verify", "limeos-storage-ready.service")
    assert (
        run(
            "systemctl", "is-enabled", "limeos-storage-ready", check=False
        ).stdout.strip()
        == "static"
    )
    assert not PLAN.exists()
    assert (
        run(
            "systemctl",
            "show",
            "limeos-storage-ready",
            "-p",
            "TimeoutStartUSec",
            "--value",
        ).stdout.strip()
        == "2min 5s"
    )
    for p in Path("/usr/lib/limeos").rglob("*"):
        assert not p.is_symlink()
        assert p.stat().st_uid == 0 and p.stat().st_mode & 0o022 == 0
    for line in run("ldd", EXECUTOR).stdout.splitlines():
        for token in line.split():
            if token.startswith("/"):
                file = Path(token)
                for p in [file, *file.parents]:
                    assert p.stat().st_uid == 0 and p.stat().st_mode & 0o022 == 0
    passed(
        "signed 0.4.0 install, protected executable/libraries and dormant bounded readiness unit"
    )

    for name in ["vdb", "vdc", "vdd", "vde"]:
        parts = "label: gpt\n,48M,L\n,48M,L\n" if name == "vdb" else "label: gpt\n,,L\n"
        run("sfdisk", f"/dev/{name}", input=parts)
    run("udevadm", "settle")
    paths = [
        ("data", "vdb1", "limeos-test-0"),
        ("data2", "vdb2", "limeos-test-0"),
        ("downloads", "vdc1", "limeos-test-1"),
        ("parity", "vdd1", "limeos-test-2"),
        ("backup", "vde1", "limeos-test-3"),
    ]
    devices = []
    for name, dev, serial in paths:
        run("mkfs.ext4", "-q", f"/dev/{dev}")
        mount = Path("/mnt") / name
        mount.mkdir()
        run("mount", f"/dev/{dev}", str(mount))
        (mount / "sentinel").write_text("synthetic storage fixture\n")
        devices.append(
            assignment(
                name,
                f"/dev/{dev}",
                str(mount),
                serial,
                role=name
                if name in ["downloads", "parity"]
                else "config_backup"
                if name == "backup"
                else "data",
            )
        )
    data, data2, downloads, parity, backup = devices
    sentinels = {
        name: hashlib.sha256(
            (Path("/mnt") / name / "sentinel").read_bytes()
        ).hexdigest()
        for name, _, _ in paths
    }
    for profile, subset in [
        ("single_disk", [data]),
        ("separate_downloads", [data, downloads]),
        ("protected_pool", [data, data2, parity, backup]),
    ]:
        write_plan(subset)
        value = json.loads(run(EXECUTOR, "storage-check", "--plan", str(PLAN)).stdout)
        assert [m["id"] for m in value["mounts"]] == [m["id"] for m in subset]
        host_root_id = next(
            int(line.split()[0])
            for line in Path("/proc/1/mountinfo").read_text().splitlines()
            if line.split()[4] == "/"
        )
        assert value["host_root_mount_id"] == host_root_id
        assert all(
            m["mount_id"] > 0 and m["device"]["major"] > 0 for m in value["mounts"]
        )
        run("systemctl", "start", "limeos-storage-ready")
        assert (
            run(
                "systemctl", "show", "limeos-storage-ready", "-p", "Result", "--value"
            ).stdout.strip()
            == "success"
        )
        passed(
            f"{profile}: UUID, inherited disk serial, exact descriptor mount and host namespace"
        )
    assert (
        not Path("/mnt/data/media").exists()
        and not Path("/mnt/data/downloads").exists()
    )
    passed("readiness does not require or create media and downloads subdirectories")

    check_error([{**data, "serial": "replacement-serial"}], "identity_mismatch")
    check_error([{**data, "filesystem": "xfs"}], "identity_mismatch")
    passed(
        "filesystem type and physical serial mismatches are refused without rewriting the plan"
    )
    boot = run("findmnt", "-n", "-o", "SOURCE", "/").stdout.strip()
    check_error([assignment("boot-alias", boot, "/mnt/boot-alias")], "boot_device")
    siblings = json.loads(
        run("lsblk", "--tree", "--json", "-o", "PATH,FSTYPE", "/dev/vda").stdout
    )["blockdevices"][0]["children"]
    boot_sibling = next(d["path"] for d in siblings if d.get("fstype") == "vfat")
    check_error(
        [
            assignment(
                "boot-sibling", boot_sibling, "/mnt/boot-sibling", filesystem="vfat"
            )
        ],
        "boot_device",
    )
    passed("root filesystem alias and another partition on the boot disk are excluded")

    run("umount", "/mnt/downloads")
    original_uuid = uuid("/dev/vdc1")
    run("tune2fs", "-U", data["filesystem_uuid"], "/dev/vdc1")
    check_error([data], "ambiguous_identity")
    run("tune2fs", "-U", original_uuid, "/dev/vdc1")
    passed(
        "fresh raw probes detect an unmounted duplicate UUID without relying on udev"
    )
    run("umount", "/mnt/data")
    run("mount", "/dev/vdc1", "/mnt/data")
    check_error([data], "identity_mismatch")
    run("umount", "/mnt/data")
    check_error([data], "not_ready")
    passed(
        "replacement disk at the same path and a leftover unmounted directory fail readiness"
    )
    write_plan([data], timeout=3)
    start = time.monotonic()
    value = run("systemctl", "start", "limeos-storage-ready", check=False, timeout=10)
    elapsed = time.monotonic() - start
    assert value.returncode != 0 and 2.5 <= elapsed <= 7, (elapsed, value.stderr)
    assert (
        run(
            "systemctl",
            "show",
            "limeos-storage-ready",
            "-p",
            "ExecMainStatus",
            "--value",
        ).stdout.strip()
        == "3"
    )
    passed(
        "an unsatisfied installed device wait fails within its three-second plan deadline"
    )

    dropin = Path("/etc/systemd/system/limeos-storage-ready.service.d")
    dropin.mkdir()
    (dropin / "watchdog.conf").write_text("[Service]\nTimeoutStartSec=4s\n")
    write_plan([data], timeout=120)
    run("systemctl", "daemon-reload")
    run("systemctl", "reset-failed", "limeos-storage-ready")
    start = time.monotonic()
    value = run("systemctl", "start", "limeos-storage-ready", check=False, timeout=10)
    watchdog_elapsed = time.monotonic() - start
    assert value.returncode != 0 and 3.5 <= watchdog_elapsed <= 8
    assert (
        run(
            "systemctl", "show", "limeos-storage-ready", "-p", "Result", "--value"
        ).stdout.strip()
        == "timeout"
    )
    (dropin / "watchdog.conf").unlink()
    run("systemctl", "daemon-reload")
    passed("systemd independently kills a longer wait at the configured watchdog bound")
    run("mount", "/dev/vdb1", "/mnt/data")
    run("mount", "/dev/vdc1", "/mnt/downloads")

    link = Path("/etc/limeos/linked-plan.json")
    link.symlink_to(PLAN)
    check_error([data], "unsafe_path", path=link)
    unsafe = Path("/etc/limeos/unsafe")
    unsafe.mkdir(mode=0o777)
    unsafe.chmod(0o777)
    (unsafe / "plan.json").write_bytes(PLAN.read_bytes())
    check_error([data], "unsafe_path", path=unsafe / "plan.json")
    Path("/mnt/data-link").symlink_to("/mnt/data")
    check_error([{**data, "mountpoint": "/mnt/data-link"}], "not_ready")
    Path("/mnt/writable").mkdir()
    Path("/mnt/writable").chmod(0o777)
    Path("/mnt/writable/data").mkdir()
    run("mount", "--bind", "/mnt/data", "/mnt/writable/data")
    check_error([{**data, "mountpoint": "/mnt/writable/data"}], "unsafe_path")
    run("umount", "/mnt/writable/data")
    passed("symlink plan/mount aliases and writable ancestors cannot verify storage")
    write_plan([data])
    original_plan = PLAN.read_bytes()
    for raw in [
        json.dumps(
            {**json.loads(original_plan), "unit": "[Service] ExecStart=/bin/sh"}
        ).encode(),
        b"{" * 65537,
    ]:
        PLAN.write_bytes(raw)
        value = run(EXECUTOR, "storage-check", "--plan", str(PLAN), check=False)
        assert (
            value.returncode == 2
            and json.loads(value.stderr)["error"] == "invalid_plan"
        )
        assert PLAN.read_bytes() == raw
    fifo = Path("/etc/limeos/plan-fifo")
    os.mkfifo(fifo, 0o600)
    start = time.monotonic()
    value = run(EXECUTOR, "storage-check", "--plan", str(fifo), check=False, timeout=2)
    assert value.returncode == 2 and time.monotonic() - start < 2
    passed(
        "free-form unit text, oversized files and a reader-blocking FIFO are refused without writes"
    )
    write_plan([data])
    value = run(
        "unshare",
        "--mount",
        EXECUTOR,
        "storage-check",
        "--plan",
        str(PLAN),
        check=False,
    )
    assert json.loads(value.stderr)["error"] == "wrong_namespace"
    passed("a private mount namespace is refused even when it sees the same filesystem")

    # Device-mapper claims its backing partition exclusively. Model a real
    # alias before mounting it, rather than attempting to map a mounted FS.
    run("umount", "/mnt/data")
    sectors = run("blockdev", "--getsz", "/dev/vdb1").stdout.strip()
    run(
        "dmsetup",
        "create",
        "limeos-data-alias",
        "--table",
        f"0 {sectors} linear /dev/vdb1 0",
    )
    run("udevadm", "settle")
    check_error([data], "ambiguous_identity")
    run("dmsetup", "remove", "limeos-data-alias")
    run("mount", "/dev/vdb1", "/mnt/data")
    passed("a live device-mapper alias cannot broaden or duplicate filesystem identity")
    for name, _, _ in paths:
        assert (
            hashlib.sha256((Path("/mnt") / name / "sentinel").read_bytes()).hexdigest()
            == sentinels[name]
        )
    passed("all synthetic data survives identity, replacement, alias and wait failures")

    # This installed payload also qualifies the previous PBKDF2 backend fix.
    hashes = json.loads(Path("/root/werkzeug-hashes.json").read_text())
    run("systemctl", "stop", "limeos-core")
    database = "/var/lib/limeos/core/core.sqlite"
    with sqlite3.connect(database) as db:
        for i, row in enumerate(hashes):
            db.execute(
                "INSERT INTO users VALUES(?,?,?, ?,1)",
                (f"legacy-{i}", f"legacy-{i}", row["hash"], '"administrator"'),
            )
    run("systemctl", "start", "limeos-core")
    time.sleep(1)
    for i, row in enumerate(hashes):
        assert http_login(f"legacy-{i}", "wrong-password") == 401
        with sqlite3.connect(database) as db:
            assert (
                db.execute(
                    "SELECT password_hash FROM users WHERE id=?", (f"legacy-{i}",)
                ).fetchone()[0]
                == row["hash"]
            )
        assert http_login(f"legacy-{i}", row["password"]) == 200
        with sqlite3.connect(database) as db:
            assert (
                db.execute(
                    "SELECT password_hash FROM users WHERE id=?", (f"legacy-{i}",)
                )
                .fetchone()[0]
                .startswith("$argon2id$")
            )
    passed(
        "installed scrypt and PBKDF2 wrong/correct login and successful-only Argon2 upgrade"
    )

    output.write_text(
        json.dumps(
            {
                "passed": checks,
                "package_version": VERSION,
                "architecture": "amd64",
                "plan_wait_seconds": elapsed,
                "watchdog_seconds": watchdog_elapsed,
                "payload_sha256": hashlib.sha256(
                    Path(EXECUTOR).read_bytes()
                ).hexdigest(),
                "scope": "disposable virtual disks and read-only verification; no storage mutations registered",
                "limitations": [
                    "Native ARM64 pending",
                    "Btrfs multi-device and FUSE-backed NTFS fail closed",
                    "Mount/unmount orchestration, dependency shutdown and runtime loss monitor pending",
                ],
            },
            indent=2,
        )
        + "\n"
    )


if __name__ == "__main__":
    main()
