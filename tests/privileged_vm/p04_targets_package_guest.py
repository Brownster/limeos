#!/usr/bin/env python3
"""Optional storage-service teardown, only in the disposable package test guest."""

import io
import json
import os
import pwd
import subprocess
import sys
import tarfile
from pathlib import Path

import p04_locks_guest as locks
import p04_planning_guest as planning


def check(repo, version="0.4.4"):
    if (
        os.geteuid() != 0
        or Path("/etc/hostname").read_text().strip() != "limeos-p01-test"
    ):
        raise SystemExit("Disposable guest required")
    run = planning.run
    run("apt-get", "install", "-y", "limeos-shadow=" + version, timeout=120)
    policy = Path("/etc/limeos/system-policy/storage-targets.json")
    if not policy.exists():
        policy.write_text(
            json.dumps(
                {
                    "version": 1,
                    "core_uid": pwd.getpwnam("limeos-core").pw_uid,
                    "allow_prepare_targets": False,
                    "managed_targets": [],
                }
            )
        )
    original = policy.read_bytes()
    run("systemctl", "start", "limeos-storage-targets")
    pid = run(
        "systemctl", "show", "limeos-storage-targets", "-p", "MainPID", "--value"
    ).stdout.strip()
    assert pid != "0"
    architecture = run("dpkg", "--print-architecture").stdout.strip()
    package = next(repo.rglob(f"limeos-shadow_{version}_{architecture}.deb"))
    control = subprocess.check_output(["dpkg-deb", "--ctrl-tarfile", str(package)])
    with tarfile.open(fileobj=io.BytesIO(control)) as archive:
        script = archive.extractfile("./prerm").read().decode()
    scoped = all(
        name not in script
        for name in [
            "limeos-storage-ready.service",
            "limeos-storage-reader.service",
            "limeos-storage-targets.service",
        ]
    )
    run("apt-get", "remove", "-y", "limeos-shadow", timeout=120)
    assert (
        run(
            "systemctl", "show", "limeos-storage-targets", "-p", "MainPID", "--value"
        ).stdout.strip()
        == pid
    )
    assert policy.read_bytes() == original
    run("/var/lib/dpkg/info/limeos.prerm", "upgrade")
    active = (
        run("systemctl", "is-active", "limeos-storage-targets", check=False).returncode
        == 0
    )
    print(
        json.dumps(
            {
                "shadow_service_names_scoped": scoped,
                "target_service_active_after_prerm": active,
            }
        ),
        flush=True,
    )
    assert scoped and not active, (
        "Optional-service teardown crossed profiles or left the root target service running"
    )
    run("/var/lib/dpkg/info/limeos.postinst", "configure")
    assert (
        run("systemctl", "is-active", "limeos-storage-targets", check=False).returncode
        != 0
    )
    assert policy.read_bytes() == original
    return "installed shadow removal preserves the active standard target service and policy; standard prerm stops every optional root service before binary replacement, and configure leaves the target service dormant"


if __name__ == "__main__":
    repo, output = map(Path, sys.argv[1:])
    locks.repository(repo)
    planning.run("apt-get", "install", "-y", "limeos=0.4.4", timeout=120)
    result = {"checks": [check(repo)], "package_version": "0.4.4"}
    output.write_text(json.dumps(result, indent=2) + "\n")
