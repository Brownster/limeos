"""Isolate Core's protected socket syscall profile without changing the real Core."""

import errno
import grp
import json
import platform
import pwd
import sqlite3
import stat
import sys
import time
from contextlib import closing
from pathlib import Path

import p04_automation_guest as fixture
import p04_core_lookup_probe as probe
from current_authority_schema import current_authority_schema

VARIANTS = {
    "baseline": (True, False),
    "no-suid": (False, False),
    "explicit": (True, True),
    "both": (False, True),
}
PREFIX = "limeos-b1-core-lookup-"
SCRIPT = Path("/usr/lib/limeos-profile-fixture/core-lookup-probe.py")
UNIT = Path("/lib/systemd/system/limeos-core.service")
run, require, sha = fixture.run, fixture.require, fixture.sha


def render_unit(source, name):
    require(name in VARIANTS, "unknown profile variant")
    expected = {
        "ExecStart": "/usr/lib/limeos/limeos-core",
        "RuntimeDirectory": "limeos-core",
        "Restart": "on-failure",
        "RestrictSUIDSGID": "yes",
        "SystemCallFilter": "@system-service",
    }
    for key, value in expected.items():
        rows = [line for line in source.splitlines() if line.startswith(key + "=")]
        require(
            rows == [key + "=" + value], "unexpected installed Core directive: " + key
        )
    suid, explicit = VARIANTS[name]
    changes = {
        "ExecStart": "/usr/bin/python3 " + str(SCRIPT),
        "RuntimeDirectory": PREFIX + name,
        "Restart": "no",
    }
    if not suid:
        changes["RestrictSUIDSGID"] = "no"
    if explicit:
        changes["SystemCallFilter"] = "@system-service openat2"
    return "".join(
        (key + "=" + changes[key] + "\n")
        if (key := line.partition("=")[0]) in changes
        else line
        for line in source.splitlines(keepends=True)
    )


def authority_counts():
    with closing(sqlite3.connect(fixture.DB.as_uri() + "?mode=ro", uri=True)) as conn:
        return {
            "schema": conn.execute("PRAGMA user_version").fetchone()[0],
            "generation": conn.execute(
                "SELECT generation FROM meta WHERE id=1"
            ).fetchone()[0],
            "users": conn.execute("SELECT count(*) FROM users").fetchone()[0],
            "jobs": conn.execute("SELECT count(*) FROM jobs").fetchone()[0],
            "schedules": conn.execute(
                "SELECT count(*) FROM automation_schedules"
            ).fetchone()[0],
        }


def parse_report(text):
    require(len(text.encode()) <= 65536, "probe journal exceeds bound")
    rows = [
        json.loads(line.removeprefix("B1_CORE_LOOKUP "))
        for line in text.splitlines()
        if line.startswith("B1_CORE_LOOKUP ")
    ]
    require(len(rows) == 1, "one complete probe report required")
    return rows[0]


def properties(unit):
    names = (
        "ActiveState",
        "SubState",
        "MainPID",
        "ExecMainStatus",
        "Result",
        "RestrictSUIDSGID",
        "SystemCallFilter",
        "MemoryMax",
        "TasksMax",
        "LimitNOFILE",
        "NoNewPrivileges",
        "User",
        "Group",
        "SupplementaryGroups",
        "CapabilityBoundingSet",
        "AmbientCapabilities",
        "ProtectProc",
        "PrivateDevices",
        "PrivateTmp",
        "ProtectSystem",
        "ProtectHome",
        "ReadWritePaths",
        "ProtectKernelTunables",
        "ProtectKernelModules",
        "ProtectKernelLogs",
        "ProtectControlGroups",
        "MemoryDenyWriteExecute",
        "RestrictAddressFamilies",
        "SystemCallArchitectures",
    )
    return dict(
        line.split("=", 1)
        for line in run(
            "systemctl",
            "show",
            unit,
            *[part for name in names for part in ("-p", name)],
            timeout=5,
        ).stdout.splitlines()
        if "=" in line
    )


def validate_matrix(evidence):
    control = evidence["control"]
    require(control["openat2_errno"] == 0, "unconfined Core-UID lookup failed")
    uid = evidence["core_uid"]
    require(
        control["uid"] == uid and control["socket"] == evidence["socket"],
        "control identity differs",
    )
    for name, (suid, _explicit) in VARIANTS.items():
        row = evidence["variants"][name]
        observed, effective = row["probe"], row["properties"]
        require(
            observed["uid"] == uid and observed["gid"] == evidence["rpc_gid"],
            "probe UID/GID differs",
        )
        require(
            set(evidence["supplementary_gids"]).issubset(observed["groups"]),
            "probe supplementary groups differ",
        )
        require(
            observed["openat2_errno"] == (errno.ENOSYS if suid else 0),
            "profile hypothesis not confirmed: " + name,
        )
        if not suid:
            require(
                observed["socket"] == evidence["socket"],
                "protected socket identity differs",
            )
        require(
            observed["status"]["NoNewPrivs"] == "1"
            and observed["status"]["Seccomp"] == "2",
            "probe privilege/filter context differs",
        )
        require(
            all(
                observed["status"][key] == "0000000000000000"
                for key in ("CapEff", "CapPrm", "CapBnd")
            ),
            "probe gained capabilities",
        )
        require(observed["nofile"] == [1024, 1024], "probe descriptor limit differs")
        require(
            effective["MemoryMax"] == "134217728"
            and effective["TasksMax"] == "32"
            and effective["LimitNOFILE"] == "1024",
            "probe service limits differ",
        )
        require(
            effective["RestrictSUIDSGID"] == ("yes" if suid else "no"),
            "effective SUID/SGID restriction differs",
        )
        require(
            effective["Result"] == "success" and effective["ExecMainStatus"] == "0",
            "probe did not complete successfully",
        )


def main(package_version=fixture.VERSION):
    fixture.disposable_guard()
    require(platform.machine() == "x86_64", "AMD64 diagnostic required")
    repo, output = map(Path, sys.argv[1:])
    evidence = {
        "summary": "running",
        "stage": "install",
        "package_version": package_version,
        "variants": {},
        "scope": "Disposable AMD64 Core-UID fixed lookup matrix; not a timer, effect or installed UI qualification",
    }
    units = []
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
        run("apt-get", "install", "-y", "limeos=" + package_version, timeout=180)
        evidence["authority_schema"] = current_authority_schema()
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
        evidence["probe_sha256"] = sha(Path(probe.__file__))
        evidence["systemd_version"] = run("systemd", "--version").stdout
        evidence["before"] = {
            "unit_sha256": sha(UNIT),
            "core": fixture.unit("limeos-core"),
            "authority": authority_counts(),
        }
        require(
            evidence["before"]["authority"]["schema"] == evidence["authority_schema"],
            "trusted schema differs",
        )
        require(int(evidence["before"]["core"]["MainPID"]) > 1, "Core is not live")
        require(
            run(
                "systemctl", "is-active", "limeos-storage-reader", check=False
            ).returncode
            != 0,
            "reader was not dormant",
        )
        run("systemctl", "start", "limeos-storage-reader")
        deadline = time.monotonic() + 6
        while not Path(probe.SOCKET).is_socket():
            require(time.monotonic() < deadline, "reader socket not ready")
            time.sleep(0.05)
        meta = Path(probe.SOCKET).lstat()
        require(
            stat.S_ISSOCK(meta.st_mode)
            and meta.st_uid == pwd.getpwnam("limeos-storage-reader").pw_uid
            and meta.st_nlink == 1
            and not meta.st_mode & 0o002,
            "unsafe reader socket",
        )
        evidence["socket"] = {
            "device": meta.st_dev,
            "inode": meta.st_ino,
            "uid": meta.st_uid,
            "gid": meta.st_gid,
            "mode": meta.st_mode,
            "links": meta.st_nlink,
        }
        evidence["core_uid"] = pwd.getpwnam("limeos-core").pw_uid
        evidence["rpc_gid"] = grp.getgrnam("limeos-rpc").gr_gid
        evidence["supplementary_gids"] = [
            grp.getgrnam(name).gr_gid
            for name in ("limeos-host-access", "limeos-container-access")
        ]
        require(not SCRIPT.parent.exists(), "probe directory already exists")
        SCRIPT.parent.mkdir(mode=0o755)
        SCRIPT.write_bytes(Path(probe.__file__).read_bytes())
        SCRIPT.chmod(0o444)
        evidence["stage"] = "control"
        evidence["control"] = parse_report(
            run(
                "runuser",
                "-u",
                "limeos-core",
                "-g",
                "limeos-rpc",
                "-G",
                "limeos-host-access",
                "-G",
                "limeos-container-access",
                "--",
                "/usr/bin/python3",
                str(SCRIPT),
                timeout=5,
            ).stdout
        )
        source = UNIT.read_text()
        evidence["installed_unit"] = source
        evidence["stage"] = "profile_matrix"
        for name in VARIANTS:
            unit_name = PREFIX + name + ".service"
            path = Path("/etc/systemd/system") / unit_name
            require(not path.exists(), "probe unit already exists")
            rendered = render_unit(source, name)
            path.write_text(rendered)
            path.chmod(0o644)
            units.append((unit_name, path))
            run("systemctl", "daemon-reload", timeout=10)
            run("systemctl", "start", unit_name, timeout=20)
            deadline = time.monotonic() + 10
            while True:
                effective = properties(unit_name)
                if effective["ActiveState"] in ("inactive", "failed"):
                    break
                require(time.monotonic() < deadline, "probe did not finish")
                time.sleep(0.05)
            journal = run(
                "journalctl",
                "-u",
                unit_name,
                "--no-pager",
                "-n",
                "10",
                "-o",
                "cat",
                timeout=5,
            ).stdout
            evidence["variants"][name] = {
                "unit": rendered,
                "properties": effective,
                "probe": parse_report(journal),
            }
            print(
                "CORE_PROFILE "
                + name
                + " errno="
                + str(evidence["variants"][name]["probe"]["openat2_errno"]),
                flush=True,
            )
        validate_matrix(evidence)
        evidence["after"] = {
            "unit_sha256": sha(UNIT),
            "core": fixture.unit("limeos-core"),
            "authority": authority_counts(),
        }
        require(
            evidence["after"] == evidence["before"], "running Core or authority changed"
        )
        require(sha(SCRIPT) == evidence["probe_sha256"], "probe bytes changed")
        evidence.update(summary="pass", stage="complete")
        print(
            "PASS installed Core profile matrix isolates RestrictSUIDSGID/openat2 without changing the running Core",
            flush=True,
        )
    except Exception as error:
        evidence.update(summary="fail", error=f"{type(error).__name__}: {error}")
        evidence["journal"] = run(
            "journalctl",
            "--no-pager",
            "-n",
            "40",
            "-o",
            "cat",
            *[arg for name, _path in units for arg in ("-u", name)],
            check=False,
            timeout=5,
        ).stdout[-24000:]
        print("CORE_PROFILE_FAILURE " + fixture.failure_packet(evidence), flush=True)
        raise
    finally:
        for name, path in units:
            run("systemctl", "stop", name, check=False, timeout=10)
            path.unlink(missing_ok=True)
        if units:
            run("systemctl", "daemon-reload", timeout=10)
        encoded = json.dumps(evidence, indent=2) + "\n"
        require(len(encoded.encode()) <= 128 * 1024, "profile result exceeds bound")
        output.write_text(encoded)


if __name__ == "__main__":
    main()
