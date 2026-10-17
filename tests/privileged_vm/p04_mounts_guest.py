# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""One real A1 mount, after the guarded approved-target suite in a disposable VM.

Call qualify(repo, output, package_version, authority_schema) after that suite.
This module never formats a disk, writes fstab, fabricates admission/receipts,
or invokes the mount runtime directly. Fixture ownership changes precede the
operation. Failure preserves uncertainty and never retries or repairs a job.
"""

import hashlib
import http.client
import json
import os
import pwd
import re
import socket
import sqlite3
import stat
import struct
import subprocess
import time
from pathlib import Path

BASE = "/api/v1/storage/mounts"
UNIT = "limeos-storage-mounts"
EXECUTOR = Path("/usr/lib/limeos/limeos-executor")
POLICY = Path("/etc/limeos/system-policy/storage-mounts.json")
CORE_DB = Path("/var/lib/limeos/core/core.sqlite")
ROOT_DB = Path("/var/lib/limeos/executors/storage/mounts.sqlite")
TARGET_DB = Path("/var/lib/limeos/executors/storage/targets.sqlite")
DOCKER_UNIT = "limeos-a1-real-docker"
DOCKER_IMAGE = Path("/usr/sbin/dockerd")
SOURCE = Path("/dev/vde1")
PREVIOUS = Path("/mnt/backup")
TARGET = Path("/mnt/approved-targets/Data")
CONFIGURATION = {
    "contract": Path("/etc/limeos/storage-contract.json"),
    "pools": Path("/etc/limeos/pools.json"),
    "protection": Path("/etc/limeos/protection.json"),
    "shares": Path("/etc/limeos/shares.json"),
}
CAPABILITIES = (1 << 2) | (1 << 19) | (1 << 21)
LIMIT = 128 * 1024
RESULT_LIMIT = 2 * 1024 * 1024
LIMITATIONS = [
    "One disposable AMD64 KVM API/Core/root-service success and replay scenario only; no Pi or ARM64 installed claim.",
    "Real Engine must report zero declared containers; live consumer and full namespace/refusal matrices remain open.",
    "Guardian death, D-state/descendant cleanup, crash inside the effect, real browser UI and pressure/peak-resource qualification remain open.",
    "Core restart after verified completion is tested; acceptance-before-Verified and pending/unknown reconciliation are not simulated.",
    "CI source/run/package provenance must be joined by the qualification owner; hashes and successful fixture execution do not alone identify a source commit.",
    "The real packaged dockerd must own its Unix listener. A transient guest-only service replaces PID1 socket activation when needed; socket-activated peer-PID support is not qualified.",
    "The same Core authority database is retained. Existing jobs, claims and target receipts are never cleared to admit this fixture.",
    "The prepared underlay is recorded before mounting, but is not held open or re-inspected beneath the resulting mount; holding it would itself be a genuine host consumer.",
]


def require(condition, message):
    if not condition:
        raise ValueError(message)


def run(*command, timeout=60, check=True):
    result = subprocess.run(
        command, check=False, text=True, capture_output=True, timeout=timeout
    )
    if check and result.returncode:
        raise RuntimeError(
            f"{command[0]} failed ({result.returncode}): {result.stderr[-2000:]}"
        )
    return result


def read(path, limit=LIMIT):
    with Path(path).open("rb") as stream:
        data = stream.read(limit + 1)
    require(len(data) <= limit, "bounded fixture input exceeded")
    return data


def pairs_unique(pairs):
    result = {}
    for key, value in pairs:
        require(key not in result, "duplicate JSON field")
        result[key] = value
    return result


def parse_json(data):
    return json.loads(data, object_pairs_hook=pairs_unique)


def digest(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def canonical(value):
    return json.dumps(value, separators=(",", ":"), ensure_ascii=False)


def text_digest(value):
    return hashlib.sha256(value.encode()).hexdigest()


def mount_text(value):
    def replace(match):
        require(
            match[0] in (r"\040", r"\011", r"\012", r"\134"), "unknown mountinfo escape"
        )
        return chr(int(match[0][1:], 8))

    result = re.sub(r"\\[0-9]{3}", replace, value)
    require("\\" not in re.sub(r"\\[0-9]{3}", "", value), "malformed mountinfo escape")
    return result


def parse_mounts(data):
    require(len(data.encode()) <= 256 * 1024, "mount table too large")
    rows = []
    seen = set()
    for line in data.splitlines():
        fields = line.split()
        require(fields.count("-") == 1, "ambiguous mountinfo separator")
        separator = fields.index("-")
        require(
            separator >= 6 and len(fields) == separator + 4, "malformed mountinfo row"
        )
        require(
            re.fullmatch(r"[0-9]+:[0-9]+", fields[2]) is not None, "bad mount device"
        )
        identifier = int(fields[0])
        require(identifier > 0 and identifier not in seen, "duplicate mount ID")
        seen.add(identifier)
        major, minor = map(int, fields[2].split(":"))
        rows.append(
            {
                "mount_id": identifier,
                "parent": int(fields[1]),
                "device": {"major": major, "minor": minor},
                "root": mount_text(fields[3]),
                "target": mount_text(fields[4]),
                "options": fields[5].split(","),
                "filesystem": fields[separator + 1],
                "source": mount_text(fields[separator + 2]),
                "super_options": fields[separator + 3].split(","),
            }
        )
    require(0 < len(rows) <= 512, "mount table count outside fixture bounds")
    return rows


def mounts():
    return parse_mounts(read("/proc/self/mountinfo", 256 * 1024).decode())


def number(value):
    return {"major": os.major(value), "minor": os.minor(value)}


def block_family(device, sysroot=Path("/sys")):
    """Bound partition/whole-device/slave/holder ancestry; unknown evidence fails."""
    sysroot = sysroot.resolve(strict=True)

    def partition_parent(path, current):
        require(
            (sysroot / "dev/block" / current).resolve(strict=True) == path
            and read(path / "dev", 32).decode().strip() == current,
            "partition identity changed",
        )
        marker = path / "partition"
        require(stat.S_ISREG(marker.lstat().st_mode), "unsafe partition attribute")
        require(
            re.fullmatch(r"[1-9][0-9]*", read(marker, 32).decode().strip()) is not None,
            "invalid partition index",
        )
        parent = path.parent
        parent_id = read(parent / "dev", 32).decode().strip()
        require(
            re.fullmatch(r"(0|[1-9][0-9]*):(0|[1-9][0-9]*)", parent_id) is not None
            and parent_id != current,
            "invalid partition parent identity",
        )
        require(
            not (parent / "partition").exists()
            and not (parent / "partition").is_symlink()
            and (sysroot / "dev/block" / parent_id).resolve(strict=True) == parent,
            "ambiguous partition parent",
        )
        return parent_id

    pending = [device]
    found = set()
    while pending:
        current = pending.pop()
        if current in found:
            continue
        require(len(found) < 16, "block ancestry exceeds fixture bounds")
        path = (sysroot / "dev/block" / current).resolve(strict=True)
        require(path.is_relative_to(sysroot.resolve()), "block ancestry escapes sysfs")
        require(
            (path / "dev").read_text().strip() == current, "block identity mismatch"
        )
        require(
            not path.name.startswith(("loop", "dm-", "md")), "unsupported block stack"
        )
        found.add(current)
        parent_id = None
        if (path / "partition").exists() or (path / "partition").is_symlink():
            parent_id = partition_parent(path, current)
            pending.append(parent_id)
        for relation in ("slaves", "holders"):
            try:
                entries = list((path / relation).iterdir())
            except FileNotFoundError:
                if (
                    relation != "slaves"
                    or parent_id is None
                    or (path / relation).is_symlink()
                ):
                    raise
                require(
                    partition_parent(path, current) == parent_id,
                    "partition ancestry changed",
                )
                entries = []
            require(len(entries) <= 8, "ambiguous block relationships")
            for entry in entries:
                pending.append((entry.resolve(strict=True) / "dev").read_text().strip())
    return found


def unchanged_file(path):
    value = path.stat()
    require(stat.S_ISREG(value.st_mode) and not path.is_symlink(), "unsafe sentinel")
    return {
        "sha256": digest(path),
        "uid": value.st_uid,
        "gid": value.st_gid,
        "mode": value.st_mode,
    }


def directory(path):
    value = path.stat()
    require(
        stat.S_ISDIR(value.st_mode) and not path.is_symlink(),
        "unsafe fixture directory",
    )
    return {
        "major": os.major(value.st_dev),
        "minor": os.minor(value.st_dev),
        "inode": value.st_ino,
        "mode": value.st_mode,
        "uid": value.st_uid,
        "gid": value.st_gid,
    }


def protected(path, regular=False):
    """Follow only the explicit resolved code path; every ancestor stays protected."""
    path = Path(path).resolve(strict=True)
    for entry in [path, *path.parents]:
        value = entry.stat()
        require(
            value.st_uid == 0 and not value.st_mode & 0o022,
            "unprotected fixture/code path",
        )
    if regular:
        require(
            path.is_file() and path.stat().st_nlink == 1,
            "unsafe executable/source file",
        )
    return path


def guard_host(repo, output):
    require(
        os.geteuid() == 0 and socket.gethostname() == "limeos-p01-test",
        "disposable guest required",
    )
    require(
        run("systemd-detect-virt", "--vm").stdout.strip() == "kvm", "KVM guest required"
    )
    require(
        run("dpkg", "--print-architecture").stdout.strip() == "amd64",
        "AMD64 guest fixture required",
    )
    require(
        repo.resolve() == Path("/opt/limeos-repo"),
        "fixed disposable package repository required",
    )
    require(
        output == Path("/root/result.json") and not output.is_symlink(),
        "fixed prior-suite result required",
    )
    protected(output, regular=True)


def guard(repo, output):
    guard_host(repo, output)
    for index, name in enumerate(("vdb", "vdc", "vdd", "vde")):
        require(
            read(f"/sys/class/block/{name}/serial").decode().strip()
            == f"limeos-test-{index}",
            "unrecognized fixture disk serial",
        )
        require(
            int(run("blockdev", "--getsize64", f"/dev/{name}").stdout)
            == 128 * 1024 * 1024,
            "fixture disk size changed",
        )
    value = SOURCE.stat()
    require(
        stat.S_ISBLK(value.st_mode) and not SOURCE.is_symlink(),
        "real fixture block source required",
    )
    selected = f"{os.major(value.st_rdev)}:{os.minor(value.st_rdev)}"
    path = Path(f"/sys/dev/block/{selected}").resolve(strict=True)
    require(
        path.name == "vde1"
        and path.parent.name == "vde"
        and (path / "partition").read_text().strip() == "1",
        "fixed separate data partition required",
    )
    require(
        sorted(p.name for p in path.parent.iterdir() if (p / "partition").is_file())
        == ["vde1"],
        "sibling partition fixture unsupported",
    )
    family = block_family(selected)
    table = mounts()
    host = parse_mounts(read("/proc/1/mountinfo", 256 * 1024).decode())
    require(
        os.path.samefile("/proc/self/ns/mnt", "/proc/1/ns/mnt"),
        "host mount namespace required",
    )
    root = [row for row in table if row["target"] == "/"]
    host_root = [row for row in host if row["target"] == "/"]
    require(
        len(root) == len(host_root) == 1 and root == host_root,
        "host root identity mismatch",
    )
    for row in table:
        if row["target"] in ("/", "/boot", "/boot/efi"):
            item = row["device"]
            require(
                not family & block_family(f"{item['major']}:{item['minor']}"),
                "selected disk shares boot backing",
            )
    swap = read("/proc/swaps").decode().splitlines()
    require(swap and swap[0].split()[0] == "Filename", "unknown swap table")
    for line in swap[1:]:
        fields = line.split()
        require(len(fields) == 5, "ambiguous swap row")
        swap_stat = Path(mount_text(fields[0])).stat()
        swap_device = (
            swap_stat.st_rdev if stat.S_ISBLK(swap_stat.st_mode) else swap_stat.st_dev
        )
        require(
            not family
            & block_family(f"{os.major(swap_device)}:{os.minor(swap_device)}"),
            "selected disk shares swap backing",
        )
    selected_mounts = [row for row in table if row["device"] == number(value.st_rdev)]
    require(
        len(selected_mounts) == 1
        and selected_mounts[0]["target"] == str(PREVIOUS)
        and selected_mounts[0]["filesystem"] == "ext4"
        and selected_mounts[0]["root"] == "/",
        "expected existing backup fixture filesystem required",
    )
    require(
        not any(row["target"].startswith(str(PREVIOUS) + "/") for row in table),
        "nested backup mount refuses",
    )
    require(
        not any(
            row["target"] == str(TARGET) or row["target"].startswith(str(TARGET) + "/")
            for row in table
        ),
        "prepared target is already mounted",
    )
    for entry in [TARGET, *TARGET.parents]:
        require(not entry.is_symlink(), "symlinked prepared target refuses")
        protected(entry)
    require(not any(TARGET.iterdir()), "target must already be prepared and empty")
    require(
        TARGET.stat().st_dev == Path("/").stat().st_dev,
        "target must be on host root underlay",
    )
    return {
        "source_device": number(value.st_rdev),
        "physical_devices": sorted(family),
        "host_root_mount_id": root[0]["mount_id"],
        "prepared_target": directory(TARGET),
        "previous_mount": selected_mounts[0],
    }


def configuration(uuid):
    device = {
        "id": "a1-data",
        "role": "data",
        "filesystem_uuid": uuid,
        "filesystem": "ext4",
        "mountpoint": str(TARGET),
        "serial": "limeos-test-3",
    }
    contract = {
        "schema_version": "1",
        "profile": "single_disk",
        "media_identity": {"uid": 1000, "gid": 1000},
        "locations": {
            "media_host": str(TARGET / "media"),
            "downloads_host": str(TARGET / "downloads"),
            "application_config_host": "/var/lib/limeos/apps",
            "backup_host": "/mnt/backup/limeos",
            "media_container": "/data/media",
            "downloads_container": "/data/downloads",
            "config_container": "/config",
        },
        "devices": [device],
    }
    protection = {
        "enabled": False,
        "drives": [],
        "excludes": [],
        "settings": {
            "blocksize": 256,
            "hashsize": 16,
            "autosave": 500,
            "nohidden": False,
            "prehash": True,
        },
        "thresholds": {"delete_threshold": 50, "update_threshold": 500},
        "scrub": {"enabled": True, "percent": 12, "age_days": 10},
        "schedule": {
            "sync_enabled": False,
            "sync_cron": "0 3 * * *",
            "scrub_enabled": False,
            "scrub_cron": "0 4 * * 0",
        },
    }
    return device, {
        "contract": contract,
        "pools": {"pools": []},
        "protection": protection,
        "shares": {"shares": []},
    }


def write_configuration(path, value):
    require(path in [*CONFIGURATION.values(), POLICY], "unexpected fixture policy path")
    protected(path.parent)
    if path.exists() or path.is_symlink():
        require(not path.is_symlink(), "symlinked fixture policy")
        protected(path, regular=True)
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC | os.O_NOFOLLOW, 0o600)
    with os.fdopen(fd, "w") as stream:
        os.fchmod(stream.fileno(), 0o600)
        stream.write(canonical(value) + "\n")
        stream.flush()
        os.fsync(stream.fileno())


def query(path, sql, parameters=()):
    require(
        path.exists() and not path.is_symlink(), "expected durable database missing"
    )
    with sqlite3.connect(f"file:{path}?mode=ro", uri=True, timeout=2) as database:
        database.execute("PRAGMA query_only=ON")
        return database.execute(sql, parameters).fetchall()


def authority_snapshot(exclude=None):
    """Read prior authority; never cancel jobs, edit claims or replace a DB."""
    jobs = query(
        CORE_DB,
        "SELECT id,state,digest,resource,revision,deadline FROM jobs ORDER BY id LIMIT 2049",
    )
    claims = query(
        CORE_DB, "SELECT resource,job FROM resource_locks ORDER BY resource LIMIT 2049"
    )
    require(
        len(jobs) <= 2048 and len(claims) <= 2048,
        "prior authority exceeds fixture bounds",
    )
    if exclude is not None:
        jobs = [row for row in jobs if row[0] != exclude]
        claims = [row for row in claims if row[1] != exclude]
    target_receipts = []
    target_locks = []
    if TARGET_DB.exists():
        headers = query(
            TARGET_DB,
            "SELECT action,digest,state,length(CAST(body AS BLOB)) FROM receipts ORDER BY action LIMIT 129",
        )
        require(len(headers) <= 128, "prior root receipt count exceeds fixture bound")
        for action, receipt_digest, state, size in headers:
            require(0 < size <= 64 * 1024, "prior root receipt exceeds fixture bound")
            body = query(
                TARGET_DB, "SELECT body FROM receipts WHERE action=?", (action,)
            )
            require(
                len(body) == 1 and len(body[0][0].encode()) == size,
                "prior root receipt changed while observing",
            )
            target_receipts.append(
                (action, receipt_digest, state, text_digest(body[0][0]))
            )
        target_locks = query(
            TARGET_DB, "SELECT resource,action FROM locks ORDER BY resource LIMIT 1025"
        )
        require(
            len(target_receipts) <= 1024 and len(target_locks) <= 1024,
            "prior root journal exceeds fixture bounds",
        )
    value = {
        "jobs": jobs,
        "claims": claims,
        "target_receipts": target_receipts,
        "target_locks": target_locks,
        "authority_database": str(CORE_DB),
        "database_separation": False,
    }
    require(
        len(canonical(value).encode()) <= 32 * 1024,
        "prior authority packet exceeds fixture bound",
    )
    return value


def authority_ready(facts, uuid):
    before = authority_snapshot()
    conflicts = {
        "storage:configuration",
        f"storage:uuid:{uuid}",
        f"storage:mount:{TARGET}",
    }
    conflicts.update(f"storage:device:{device}" for device in facts["physical_devices"])
    require(
        not any(row[0] in conflicts for row in before["claims"]),
        "prior uncertain Core claims conflict with A1; preserve them",
    )
    pending = query(
        CORE_DB,
        "SELECT id,state,json_extract(intent,'$.operation') FROM jobs WHERE state IN ('queued','running','verifying','outcome_unknown','needs_intervention') LIMIT 2049",
    )
    require(len(pending) <= 2048, "pending authority exceeds fixture bounds")
    require(
        not any(str(row[2]).startswith("storage_") for row in pending),
        "prior pending storage work must remain intact; A1 refuses this fixture",
    )
    require(
        not before["target_locks"],
        "prior root target claims remain; A1 will not clear or replace them",
    )
    return before


def preserved_authority(before, after):
    require(before["claims"] == after["claims"], "prior unresolved Core claims changed")
    require(
        before["target_receipts"] == after["target_receipts"]
        and before["target_locks"] == after["target_locks"],
        "prior root target authority changed",
    )
    require(
        len(before["jobs"]) == len(after["jobs"]), "prior Core job membership changed"
    )
    active = {"queued", "running", "verifying", "outcome_unknown", "needs_intervention"}
    for old, new in zip(before["jobs"], after["jobs"]):
        require(
            (old[0], *old[2:]) == (new[0], *new[2:]),
            "prior immutable Core intent/resource identity changed",
        )
        # Store startup legitimately moves running/verifying jobs to
        # needs_intervention and writes generation/recovery audit. Those are
        # not DB corruption, claim release or effect verification.
        require(
            new[1] in active if old[1] in active else old[1] == new[1],
            "prior unresolved barrier was converted to a terminal result",
        )


def stop_prior_target_writer():
    before = run(
        "systemctl",
        "show",
        "limeos-storage-targets",
        "-p",
        "MainPID",
        "-p",
        "ControlGroup",
    ).stdout
    properties = dict(row.split("=", 1) for row in before.splitlines())
    run("systemctl", "stop", "limeos-storage-targets")
    require(
        run(
            "systemctl", "show", "limeos-storage-targets", "-p", "MainPID", "--value"
        ).stdout.strip()
        == "0",
        "prior target writer still has a main process",
    )
    group = properties.get("ControlGroup", "")
    if group:
        require(
            group.startswith("/system.slice/") and ".." not in group.split("/"),
            "unexpected prior writer cgroup",
        )
        events = Path("/sys/fs/cgroup") / group.lstrip("/") / "cgroup.events"
        if events.exists():
            require(
                "populated 0" in read(events).decode().splitlines(),
                "prior writer descendants remain; preserve prior authority",
            )
    return {
        "before": properties,
        "main_pid_after": 0,
        "owned_cgroup_empty_or_removed": True,
    }


def api(path, method="GET", body=None, cookie=None, csrf=None):
    connection = http.client.HTTPConnection("127.0.0.1", 8003, timeout=40)
    headers = {"Origin": "https://localhost", "Content-Type": "application/json"}
    if cookie:
        headers["Cookie"] = cookie
    if csrf:
        headers["X-CSRF-Token"] = csrf
    try:
        connection.request(
            method, path, None if body is None else canonical(body), headers
        )
        response = connection.getresponse()
        data = response.read(LIMIT + 1)
        require(len(data) <= LIMIT, "API response exceeds fixture bound")
        return (
            response.status,
            dict(response.getheaders()),
            parse_json(data) if data else None,
        )
    finally:
        connection.close()


def expect(status, response):
    require(
        response[0] == status,
        f"unexpected API status {response[0]}, expected {status}: {canonical(response[2])[:1000]}",
    )
    return response[2]


def refusal(status, code, response):
    body = expect(status, response)
    require(
        isinstance(body, dict)
        and body.get("code") == code
        and body.get("retry") is False,
        "expected exact non-retryable API refusal cause",
    )
    return {"status": status, "code": code, "retry": False}


def unmounted_source_digest(facts):
    """Read only the guarded partition, with fixed-size streaming and a hard cap."""
    size = int(read("/sys/class/block/vde1/size", 32).decode()) * 512
    require(0 < size <= 128 * 1024 * 1024, "partition exceeds fixture read bound")
    descriptor = os.open(SOURCE, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC)
    with os.fdopen(descriptor, "rb", buffering=0) as stream:
        identity = os.fstat(stream.fileno())
        require(
            stat.S_ISBLK(identity.st_mode)
            and number(identity.st_rdev) == facts["source_device"],
            "pre-effect source identity changed",
        )
        checksum = hashlib.sha256()
        remaining = size
        while remaining:
            chunk = stream.read(min(64 * 1024, remaining))
            require(chunk, "pre-effect source became truncated")
            checksum.update(chunk)
            remaining -= len(chunk)
        require(not stream.read(1), "pre-effect source size changed")
    return {"bytes": size, "sha256": checksum.hexdigest()}


def pre_effect_snapshot(facts):
    table = mounts()
    require(
        not any(
            row["device"] == facts["source_device"]
            or row["target"] == str(TARGET)
            or row["target"].startswith(str(TARGET) + "/")
            for row in table
        ),
        "pre-effect refusal mounted the source or target",
    )
    require(not any(TARGET.iterdir()), "pre-effect target is no longer empty")
    root_counts = [0, 0]
    if ROOT_DB.exists():
        root_counts = [
            query(ROOT_DB, f"SELECT count(*) FROM {name}")[0][0]
            for name in ("receipts", "locks")
        ]
    require(root_counts == [0, 0], "pre-effect root action receipt/claim appeared")
    return {
        "authority": authority_snapshot(),
        "root_receipts_and_claims": root_counts,
        "mount_table_sha256": text_digest(canonical(table)),
        "source": unmounted_source_digest(facts),
        "target": directory(TARGET),
        "fstab_sha256": digest(Path("/etc/fstab")),
        "configuration_sha256": {
            str(path): digest(path) for path in [*CONFIGURATION.values(), POLICY]
        },
        "other_data": {
            name: {
                "root": directory(Path("/mnt") / name),
                "sentinel": unchanged_file(Path("/mnt") / name / "sentinel"),
            }
            for name in ("data", "data2", "downloads", "parity")
        },
    }


def pre_effect_unchanged(before, after):
    preserved_authority(before["authority"], after["authority"])
    require(
        {key: value for key, value in before.items() if key != "authority"}
        == {key: value for key, value in after.items() if key != "authority"},
        "pre-effect refusal changed mount, fstab, target, configuration or data",
    )


def pre_effect_refusals(facts, device, cookie, csrf, prior_authority):
    """Use real API authority; leave the cancelled plan intact and create no job."""
    before = pre_effect_snapshot(facts)
    preserved_authority(prior_authority, before["authority"])
    checks = []

    def record(name, responses):
        pre_effect_unchanged(before, pre_effect_snapshot(facts))
        checks.append({"case": name, "responses": responses, "no_effect": True})

    record(
        "extra_path_field",
        [
            refusal(
                400,
                "invalid_input",
                api(
                    BASE + "/plans",
                    "POST",
                    {"target_id": device["id"], "mountpoint": "/mnt/unapproved"},
                    cookie,
                    csrf,
                ),
            )
        ],
    )
    proposal = expect(
        201, api(BASE + "/plans", "POST", {"target_id": device["id"]}, cookie, csrf)
    )
    require(
        proposal["plan"]["effect"]["device"] == device
        and proposal["digest"] == text_digest(canonical(proposal["plan"]))
        and re.fullmatch(r"[0-9a-f]{64}", proposal["digest"]),
        "negative case requires a genuine reviewed plan",
    )
    plan_path = BASE + f"/plans/{proposal['plan']['id']}"
    changed = ("0" if proposal["digest"][0] != "0" else "1") + proposal["digest"][1:]
    record(
        "tampered_approval_digest",
        [
            refusal(
                409,
                "conflict",
                api(plan_path + "/approval", "POST", {"digest": changed}, cookie, csrf),
            )
        ],
    )
    approval = expect(
        200,
        api(
            plan_path + "/approval",
            "POST",
            {"digest": proposal["digest"]},
            cookie,
            csrf,
        ),
    )
    require(
        isinstance(approval, dict)
        and isinstance(approval.get("token"), str)
        and re.fullmatch(r"[0-9a-f]{64}", approval["token"]),
        "negative case requires genuine approval authority",
    )
    expect(204, api(plan_path + "/cancel", "POST", cookie=cookie, csrf=csrf))
    record(
        "cancelled_approved_plan",
        [
            refusal(
                404,
                "not_found",
                api(
                    plan_path + "/approval",
                    "POST",
                    {"digest": proposal["digest"]},
                    cookie,
                    csrf,
                ),
            ),
            refusal(
                404,
                "not_found",
                api(
                    BASE + "/jobs",
                    "POST",
                    {
                        "proposal": proposal,
                        "approval": approval["token"],
                        "idempotency_key": "a1-cancelled-plan-must-not-queue",
                    },
                    cookie,
                    csrf,
                ),
            ),
        ],
    )
    require(
        query(
            CORE_DB,
            "SELECT canceled,job,approval_digest,digest FROM storage_mount_plans WHERE id=?",
            (proposal["plan"]["id"],),
        )
        == [(1, None, None, proposal["digest"])],
        "cancelled plan was removed, reused or retained approval authority",
    )
    # Plan/audit records may grow; jobs, claims, receipts and immutable data may not.
    return {
        "cases": checks,
        "baseline_without_authority": {
            key: value for key, value in before.items() if key != "authority"
        },
        "prior_jobs": len(before["authority"]["jobs"]),
        "prior_claims": len(before["authority"]["claims"]),
        "cancelled_plan": proposal["plan"]["id"],
        "cancelled_plan_preserved": True,
    }


def login():
    passwords = parse_json(read("/root/werkzeug-hashes.json"))
    response = api(
        "/api/v1/auth/login",
        "POST",
        {"username": "legacy-0", "password": passwords[0]["password"]},
    )
    session = expect(200, response)
    require(
        session["principal"]["role"] == "administrator",
        "actual administrator session required",
    )
    return (
        response[1]["set-cookie"].split(";", 1)[0],
        session["csrf_token"],
        session["principal"],
    )


def engine_peer():
    peer = socket.socket(socket.AF_UNIX)
    try:
        peer.settimeout(3)
        peer.connect("/run/docker.sock")
        pid, uid, gid = struct.unpack(
            "3i", peer.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, 12)
        )
    finally:
        peer.close()
    require(pid > 0 and uid == 0, "real root Engine peer required")
    return {"pid": pid, "uid": uid, "gid": gid}


def actual_docker():
    run("apt-get", "install", "-y", "docker.io", timeout=180)
    run("systemctl", "start", "docker.socket", "docker.service")
    initial_peer = engine_peer()
    declared = run(
        "docker",
        "--host",
        "unix:///run/docker.sock",
        "ps",
        "--all",
        "--quiet",
        "--no-trunc",
    ).stdout.splitlines()
    require(
        not declared,
        "this first positive fixture requires genuine zero-container Engine inventory",
    )
    docker_unit = "docker"
    if initial_peer["pid"] == 1:
        # This changes only the disposable fixture's real packaged daemon
        # launch. Neither a synthetic server nor a producer trust exception is
        # substituted for the public Engine acquisition path.
        run("systemctl", "stop", "docker.service", "docker.socket")
        require(
            run("systemctl", "is-active", "docker.service", check=False).returncode != 0
            and run("systemctl", "is-active", "docker.socket", check=False).returncode
            != 0,
            "socket-activated Engine remains active",
        )
        # systemd may retain the stopped socket's filesystem name. The real
        # dockerd replaces its own stale socket; this fixture does not unlink
        # an unauthenticated path or keep PID1's open listener in use.
        if Path("/run/docker.sock").exists():
            old = Path("/run/docker.sock").lstat()
            require(
                stat.S_ISSOCK(old.st_mode) and old.st_uid == 0 and old.st_nlink == 1,
                "unsafe stale Docker socket",
            )
        protected(DOCKER_IMAGE, regular=True)
        run("systemctl", "start", "containerd")
        run(
            "systemd-run",
            "--unit=" + DOCKER_UNIT,
            "--service-type=exec",
            "--property=Delegate=yes",
            "--property=Restart=no",
            "--property=KillMode=control-group",
            "--property=TimeoutStopSec=10s",
            str(DOCKER_IMAGE),
            "--host=unix:///run/docker.sock",
            "--containerd=/run/containerd/containerd.sock",
            "--bridge=none",
            "--iptables=false",
            "--ip-forward=false",
            "--ip-masq=false",
        )
        docker_unit = DOCKER_UNIT
        deadline = time.monotonic() + 20
        while True:
            try:
                ready_peer = engine_peer()
                require(
                    ready_peer["pid"] > 1,
                    "PID1 listener remains after socket deactivation",
                )
                break
            except (FileNotFoundError, ConnectionRefusedError):
                require(
                    time.monotonic() < deadline,
                    "real dockerd listener did not become ready",
                )
                time.sleep(0.1)
    peer = engine_peer()
    daemon = int(
        run("systemctl", "show", docker_unit, "-p", "MainPID", "--value").stdout
    )
    require(
        daemon > 1 and peer["pid"] == daemon,
        "real dockerd must own this fixture listener; PID1 socket activation is not qualified",
    )
    require(
        Path(f"/proc/{daemon}/exe").resolve(strict=True)
        == protected(DOCKER_IMAGE, regular=True),
        "fixture listener is not the packaged dockerd image",
    )
    require(
        os.path.samefile(f"/proc/{daemon}/ns/mnt", "/proc/1/ns/mnt"),
        "real Engine must use the guest host mount namespace",
    )
    identities = parse_json(
        run(
            "docker",
            "--host",
            "unix:///run/docker.sock",
            "info",
            "--format",
            "{{json .}}",
        ).stdout
    )
    require(
        identities.get("ID") and identities.get("DockerRootDir"),
        "actual Engine identity required",
    )
    declared = run(
        "docker",
        "--host",
        "unix:///run/docker.sock",
        "ps",
        "--all",
        "--quiet",
        "--no-trunc",
    ).stdout.splitlines()
    require(
        not declared,
        "this first positive fixture requires genuine zero-container Engine inventory",
    )
    return {
        "peer": peer,
        "initial_peer": initial_peer,
        "unit": docker_unit,
        "dockerd_sha256": digest(DOCKER_IMAGE),
        "docker_package_version": run(
            "dpkg-query", "-W", "-f=${Version}", "docker.io"
        ).stdout,
        "listener_fixture": "real packaged dockerd-owned Unix socket; socket activation excluded",
        "id": identities["ID"],
        "root": identities["DockerRootDir"],
        "server_version": identities.get("ServerVersion"),
        "declared_containers": declared,
    }


def service_identity():
    pid = int(run("systemctl", "show", UNIT, "-p", "MainPID", "--value").stdout)
    require(pid > 1, "real root service process required")
    status = dict(
        line.split(":", 1)
        for line in read(f"/proc/{pid}/status").decode().splitlines()
        if ":" in line
    )
    require(
        status["Uid"].split() == ["0"] * 4,
        "installed mount service is not actually root",
    )
    for field in ("CapEff", "CapBnd", "CapAmb"):
        require(
            int(status[field].strip(), 16) == CAPABILITIES,
            "dedicated service capability mismatch",
        )
    require(status["NoNewPrivs"].strip() == "1", "NoNewPrivileges required")
    require(
        os.path.samefile(f"/proc/{pid}/ns/mnt", "/proc/1/ns/mnt"),
        "service is not in host mount namespace",
    )
    require(
        os.path.samefile(f"/proc/{pid}/root", "/proc/1/root"),
        "service root differs from PID1",
    )
    require(
        Path(f"/proc/{pid}/exe").resolve(strict=True) == EXECUTOR,
        "wrong installed executor image",
    )
    peer = socket.socket(socket.AF_UNIX)
    try:
        peer.settimeout(3)
        peer.connect("/run/limeos-storage-mounts/executor.sock")
        peer_pid, peer_uid, _ = struct.unpack(
            "3i", peer.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, 12)
        )
        require(
            peer_pid == pid and peer_uid == 0,
            "actual mount RPC listener differs from the installed root service",
        )
    finally:
        peer.close()
    properties = run(
        "systemctl",
        "show",
        UNIT,
        "-p",
        "ControlGroup",
        "-p",
        "LimitNOFILE",
        "-p",
        "MemoryMax",
        "-p",
        "TasksMax",
        "-p",
        "KillMode",
        "-p",
        "SendSIGKILL",
        "-p",
        "Restart",
    ).stdout
    values = dict(row.split("=", 1) for row in properties.splitlines())
    require(
        values["KillMode"] == "control-group"
        and values["SendSIGKILL"] == "yes"
        and values["Restart"] == "no",
        "unsafe helper teardown profile",
    )
    require(
        values["LimitNOFILE"] == "256"
        and values["MemoryMax"] == str(64 * 1024 * 1024)
        and values["TasksMax"] == "16",
        "service limits changed",
    )
    core_pid = int(
        run("systemctl", "show", "limeos-core", "-p", "MainPID", "--value").stdout
    )
    core_status = read(f"/proc/{core_pid}/status").decode()
    core_uid = re.search(r"(?m)^Uid:\s*(.+)$", core_status)
    require(
        core_uid is not None
        and core_uid[1].split() == [str(pwd.getpwnam("limeos-core").pw_uid)] * 4,
        "Core UID differs from the root ceiling's actual peer",
    )
    core_cap = re.search(r"(?m)^CapEff:\s*([0-9a-f]+)$", core_status)
    require(
        core_cap is not None and not int(core_cap[1], 16) & (1 << 21),
        "Core gained mount capability",
    )
    return {
        "pid": pid,
        "status": {
            key: status[key].strip()
            for key in ("CapEff", "CapBnd", "CapAmb", "NoNewPrivs", "Threads", "VmRSS")
        },
        "properties": values,
        "fd_count_after_start": len(list(Path(f"/proc/{pid}/fd").iterdir())),
    }


def trusted_images():
    paths = [EXECUTOR, Path("/usr/bin/mount")]
    for image in list(paths):
        protected(image, regular=True)
        value = run("ldd", str(image))
        for line in value.stdout.splitlines():
            paths.extend(Path(word) for word in line.split() if word.startswith("/"))
    return {
        str(path.resolve(strict=True)): digest(protected(path, regular=True))
        for path in paths
    }


def mounted_snapshot(facts, uuid):
    source = SOURCE.stat()
    require(
        stat.S_ISBLK(source.st_mode)
        and not SOURCE.is_symlink()
        and number(source.st_rdev) == facts["source_device"],
        "source block rdev changed",
    )
    table = mounts()
    selected = [row for row in table if row["target"] == str(TARGET)]
    require(len(selected) == 1, "exactly one named resulting mount required")
    row = selected[0]
    require(
        row["device"] == facts["source_device"]
        and row["filesystem"] == "ext4"
        and row["root"] == "/",
        "mounted filesystem identity mismatch",
    )
    require(
        {"rw", "nosuid", "nodev"} <= set(row["options"])
        and not {"ro", "suid", "dev"} & set(row["options"]),
        "required mount flags absent",
    )
    actual_uuid = run(
        "blkid", "-p", "-s", "UUID", "-o", "value", str(SOURCE)
    ).stdout.strip()
    require(actual_uuid == uuid, "source UUID changed")
    roots = [item for item in table if item["target"] == "/"]
    require(len(roots) == 1, "exactly one host root mount required")
    root = next(iter(roots))
    require(root["mount_id"] == facts["host_root_mount_id"], "host root changed")
    identity = directory(TARGET)
    require(
        {"major": identity["major"], "minor": identity["minor"]} == row["device"],
        "named target device differs from mount table",
    )
    identity["mount_id"] = row["mount_id"]
    return {
        "filesystem_uuid": uuid,
        "device": row["device"],
        "mountpoint": str(TARGET),
        "filesystem": "ext4",
        "filesystem_root": "/",
        "writable": True,
        "nosuid": True,
        "nodev": True,
        "mount_id": row["mount_id"],
        "host_root_mount_id": facts["host_root_mount_id"],
        "target": identity,
    }


def resources(effect):
    return sorted(
        [
            "storage:configuration",
            f"storage:uuid:{effect['device']['filesystem_uuid']}",
            f"storage:mount:{effect['device']['mountpoint']}",
        ]
        + [
            f"storage:device:{device['major']}:{device['minor']}"
            for device in effect["physical_devices"]
        ]
    )


def durable_result(job, proposal, snapshot, executor_digest):
    action = job["id"]
    effect = proposal["plan"]["effect"]
    require(
        action == effect["action"]
        and job["plan"] == proposal["plan"]
        and job["state"] == "succeeded",
        "wrong terminal job binding",
    )
    rows = query(
        ROOT_DB, "SELECT digest,state,body FROM receipts WHERE action=?", (action,)
    )
    require(len(rows) == 1, "exact root receipt required")
    digest_value, state, body = rows[0]
    receipt = parse_json(body)
    require(
        state == receipt["state"] == "verified" and receipt["plan"] == effect,
        "root receipt plan/state mismatch",
    )
    require(
        digest_value == receipt["digest"] == text_digest(canonical(effect)),
        "root effect digest mismatch",
    )
    require(
        receipt["after"] == snapshot,
        "independent real mount proof differs from root receipt",
    )
    stopped = receipt["quiescence"]
    require(
        stopped["state"] == "reaped"
        and stopped["outcome"] == {"status": "exited", "code": 0},
        "genuine root helper success/quiescence missing",
    )
    require(
        stopped["helper"]["image_digest"] == executor_digest
        and stopped["helper"]["pid"] > 1,
        "guardian image binding differs",
    )
    row = query(CORE_DB, "SELECT intent,digest,state FROM jobs WHERE id=?", (action,))
    require(
        len(row) == 1 and row[0][2] == "succeeded",
        "durable Core terminal result missing",
    )
    require(
        parse_json(row[0][0])
        == {"operation": "storage_mount", "plan": proposal["plan"]}
        and row[0][1] == text_digest(row[0][0]),
        "Core immutable intent binding differs",
    )
    results = query(
        CORE_DB,
        "SELECT receipt,verification FROM storage_mount_results WHERE job=?",
        (action,),
    )
    require(
        len(results) == 1
        and parse_json(results[0][0]) == receipt
        and parse_json(results[0][1]) == [snapshot, None],
        "Core independent result binding differs",
    )
    required = [
        item[0]
        for item in query(
            CORE_DB,
            "SELECT resource FROM job_resources WHERE job=? ORDER BY resource",
            (action,),
        )
    ]
    require(
        required == resources(effect), "complete canonical Core resource set differs"
    )
    require(
        not query(
            CORE_DB, "SELECT resource FROM resource_locks WHERE job=?", (action,)
        ),
        "Core claims released without completed verification",
    )
    require(
        not query(ROOT_DB, "SELECT resource FROM locks WHERE action=?", (action,)),
        "root claims remain after verified completion",
    )
    events = [
        parse_json(item[0])
        for item in query(
            CORE_DB, "SELECT kind FROM events WHERE job=? ORDER BY cursor", (action,)
        )
    ]
    require(
        {"kind": "job_transition", "state": "running"} in events
        and {"kind": "job_transition", "state": "succeeded"} in events,
        "durable dispatch/success audit missing",
    )
    return {"receipt": receipt, "resources": required, "events": events}


def publish(output, evidence, result):
    evidence["a1_mount"] = result
    require(len(canonical(result).encode()) <= LIMIT, "A1 result exceeds fixture bound")
    encoded = json.dumps(evidence, indent=2).encode() + b"\n"
    require(len(encoded) <= RESULT_LIMIT, "combined guest result exceeds fixture bound")
    temporary = output.with_name(f".result-a1-{os.getpid()}.json")
    fd = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    with os.fdopen(fd, "wb") as stream:
        stream.write(encoded)
        stream.flush()
        os.fsync(stream.fileno())
    os.replace(temporary, output)
    directory_fd = os.open(output.parent, os.O_RDONLY | os.O_DIRECTORY | os.O_CLOEXEC)
    try:
        os.fsync(directory_fd)
    finally:
        os.close(directory_fd)


def failure_packet(result):
    """The runner retains failure stdout even when it cannot copy result.json."""
    encoded = canonical(result)
    if len(encoded.encode()) <= LIMIT:
        return encoded
    return canonical(
        {
            key: result.get(key)
            for key in (
                "summary",
                "stage",
                "error",
                "publication_error",
                "action",
                "fixture_sha256",
                "source_commit",
            )
        }
        | {
            "details": "detailed packet exceeds stdout limit; successful detailed persistence is not assumed"
        }
    )


def qualify(repo, output, package_version, authority_schema):
    """Run once on the prior suite's exact disposable fixture; no effect retry."""
    repo, output = Path(repo), Path(output)
    result = {
        "summary": "incomplete",
        "stage": "guest_guard",
        "started": time.time(),
        "source_commit": None,
        "source_identity": "whole-source SHA not supplied; join exact package/binary hashes to external CI provenance",
        "authority_schema": authority_schema,
        "package_version": package_version,
        "fixture_sha256": digest(Path(__file__)),
        "limitations": LIMITATIONS,
    }
    evidence = None
    started = False
    failure = None
    try:
        guard_host(repo, output)
        evidence = parse_json(read(output, 1024 * 1024))
        require(
            isinstance(evidence, dict) and "a1_mount" not in evidence,
            "new result required; never overwrite prior A1 evidence",
        )
        result["prior_result_sha256"] = digest(output)
        facts = guard(repo, output)  # Every fixture write follows these guards.
        result["fixture"] = facts
        result["stage"] = "installed_identity"
        from current_authority_schema import SOURCE_NAME, current_authority_schema

        schema_source = Path(__file__).with_name(SOURCE_NAME)
        protected(schema_source, regular=True)
        require(
            type(authority_schema) is int
            and authority_schema >= 10
            and current_authority_schema() == authority_schema,
            "candidate source schema expectation missing or inconsistent",
        )
        result["schema_source_sha256"] = digest(schema_source)
        require(
            query(CORE_DB, "PRAGMA user_version") == [(authority_schema,)],
            "installed schema differs from actual supplied candidate source",
        )
        if ROOT_DB.exists():
            require(
                query(ROOT_DB, "SELECT count(*) FROM receipts") == [(0,)]
                and query(ROOT_DB, "SELECT count(*) FROM locks") == [(0,)],
                "existing A1 journal/claims must be preserved; fresh fixture required",
            )
        uuid = run(
            "blkid", "-p", "-s", "UUID", "-o", "value", str(SOURCE)
        ).stdout.strip()
        require(re.fullmatch(r"[0-9a-fA-F-]{16,64}", uuid), "actual ext4 UUID required")
        require(
            run("blkid", "-p", "-s", "TYPE", "-o", "value", str(SOURCE)).stdout.strip()
            == "ext4",
            "existing ext4 filesystem required",
        )
        result["authority_before"] = authority_ready(facts, uuid)
        result["fstab_before"] = digest(Path("/etc/fstab"))
        result["data_before"] = {
            name: unchanged_file(Path("/mnt") / name / "sentinel")
            for name in ("data", "data2", "downloads", "parity", "backup")
        }
        result["stage"] = "real_engine_fixture"
        result["docker"] = actual_docker()
        result["images_sha256"] = trusted_images()
        architecture = run("dpkg", "--print-architecture").stdout.strip()
        require(architecture == "amd64", "first installed fixture is hosted AMD64 only")
        require(
            run("dpkg-query", "-W", "-f=${Version}", "limeos").stdout
            == package_version,
            "installed candidate version differs",
        )
        package = list(repo.rglob(f"limeos_{package_version}_{architecture}.deb"))
        require(
            len(package) == 1 and not package[0].is_symlink(),
            "exact signed-repository candidate package required",
        )
        run("dpkg", "--verify", "limeos")
        result["package_sha256"] = digest(package[0])
        result["binary_sha256"] = {
            name: digest(Path("/usr/lib/limeos") / name)
            for name in (
                "limeos-core",
                "limeos-executor",
                "limeos-password-worker",
                "limeosctl",
            )
        }
        result["unit_sha256"] = digest(
            protected(
                Path("/lib/systemd/system/limeos-storage-mounts.service"), regular=True
            )
        )
        require(
            run("systemctl", "is-enabled", UNIT, check=False).stdout.strip()
            == "static",
            "mount effect unit must remain opt-in",
        )
        require(
            run("systemctl", "is-active", UNIT, check=False).returncode != 0
            and not POLICY.exists(),
            "prior fixture must not provision/activate mount authority",
        )
        result["filesystem_root_before_preparation"] = directory(PREVIOUS)
        # Fixture setup only. The mounted filesystem root intentionally differs
        # from the prepared underlying target. The approved operation cannot chown.
        os.chown(PREVIOUS, 1000, 1000)
        os.chmod(PREVIOUS, 0o755)
        prepared_root = directory(PREVIOUS)
        run("umount", str(PREVIOUS))  # Only the guarded prior fixture, before approval.
        require(
            not any(row["device"] == facts["source_device"] for row in mounts()),
            "data filesystem still mounted",
        )
        device, documents = configuration(uuid)
        for name, document in documents.items():
            write_configuration(CONFIGURATION[name], document)
        ceiling = {
            "version": 1,
            "core_uid": pwd.getpwnam("limeos-core").pw_uid,
            "allow_mount": True,
            "managed_targets": [
                {
                    "target_id": device["id"],
                    "filesystem_uuid": uuid,
                    "mountpoint": str(TARGET),
                }
            ],
        }
        write_configuration(POLICY, ceiling)
        result["configuration_sha256"] = {
            str(path): digest(path) for path in [*CONFIGURATION.values(), POLICY]
        }
        require(
            digest(Path("/etc/fstab")) == result["fstab_before"],
            "fixture preparation changed original fstab",
        )
        result["prepared_filesystem_root"] = prepared_root
        result["previous_writer"] = stop_prior_target_writer()
        preserved_authority(result["authority_before"], authority_snapshot())
        result["stage"] = "installed_mount_service"
        started = True
        run("systemctl", "start", UNIT)
        # The first live catalog read proves socket readiness; failure does not
        # trigger another plan/approval/queue or bypass the production collectors.
        cookie, csrf, principal = login()
        deadline = time.monotonic() + 10
        while not Path("/run/limeos-storage-mounts/executor.sock").is_socket():
            require(
                time.monotonic() < deadline,
                "mount service listener did not become ready",
            )
            time.sleep(0.1)
        result["service"] = service_identity()
        require(
            expect(200, api(BASE + "/targets", cookie=cookie)) == {"targets": [device]},
            "catalog differs from exact managed selection",
        )
        result["stage"] = "pre_effect_api_refusals"
        result["pre_effect"] = pre_effect_refusals(
            facts, device, cookie, csrf, result["authority_before"]
        )
        result["stage"] = "human_plan_approval_queue"
        require(
            api(
                BASE + "/plans", "POST", {"target_id": device["id"]}, cookie, "bad-csrf"
            )[0]
            == 403,
            "CSRF refusal missing",
        )
        proposal = expect(
            201, api(BASE + "/plans", "POST", {"target_id": device["id"]}, cookie, csrf)
        )
        effect = proposal["plan"]["effect"]
        require(
            proposal["plan"]["principal"] == principal["id"]
            and effect["operation"] == "storage.mount"
            and effect["version"] == 1
            and effect["device"] == device,
            "reviewed plan identity differs",
        )
        require(
            effect["target"]["existing"]
            == (facts["prepared_target"] | {"mount_id": facts["host_root_mount_id"]}),
            "reviewed empty underlay differs from the prepared fixture",
        )
        require(
            sorted(
                f"{item['major']}:{item['minor']}"
                for item in effect["physical_devices"]
            )
            == facts["physical_devices"],
            "reviewed complete physical set differs from actual disk ancestry",
        )
        require(
            proposal["digest"] == text_digest(canonical(proposal["plan"])),
            "review digest mismatch",
        )
        approval = expect(
            200,
            api(
                BASE + f"/plans/{proposal['plan']['id']}/approval",
                "POST",
                {"digest": proposal["digest"]},
                cookie,
                csrf,
            ),
        )
        queue = {
            "proposal": proposal,
            "approval": approval["token"],
            "idempotency_key": "a1-mount-positive-once",
        }
        job = expect(202, api(BASE + "/jobs", "POST", queue, cookie, csrf))
        result["action"] = job["id"]
        result["stage"] = "await_terminal_receipt"
        publish(output, evidence, result)
        deadline = time.monotonic() + 60
        while job["state"] in ("queued", "running", "verifying"):
            require(
                time.monotonic() < deadline,
                "approved mount did not complete in fixture bound",
            )
            time.sleep(0.2)
            job = expect(200, api(BASE + f"/jobs/{job['id']}", cookie=cookie))
        result["job"] = job
        require(
            job["state"] == "succeeded",
            "mount job is uncertain/refused; preserve claims and do not retry",
        )
        snapshot = mounted_snapshot(facts, uuid)
        require(
            {key: snapshot["target"][key] for key in ("uid", "gid", "mode", "inode")}
            == {key: prepared_root[key] for key in ("uid", "gid", "mode", "inode")},
            "operation changed existing filesystem root ownership/mode/identity",
        )
        result["durable"] = durable_result(
            job, proposal, snapshot, result["binary_sha256"]["limeos-executor"]
        )
        result["independent_mounted"] = snapshot
        preserved_authority(result["authority_before"], authority_snapshot(job["id"]))
        result["stage"] = "same_key_replay_and_core_restart"
        before_replay = query(
            CORE_DB, "SELECT count(*) FROM events WHERE job=?", (job["id"],)
        )
        replay = expect(202, api(BASE + "/jobs", "POST", queue, cookie, csrf))
        require(
            replay == job and mounted_snapshot(facts, uuid) == snapshot,
            "lost-response replay changed job or mount identity",
        )
        require(
            query(CORE_DB, "SELECT count(*) FROM events WHERE job=?", (job["id"],))
            == before_replay,
            "queue replay wrote another audit/dispatch",
        )
        require(
            durable_result(
                job, proposal, snapshot, result["binary_sha256"]["limeos-executor"]
            )
            == result["durable"],
            "queue replay changed root receipt/resources/audit",
        )
        run("systemctl", "restart", "limeos-core")
        deadline = time.monotonic() + 30
        while run("/usr/lib/limeos/limeosctl", "status", check=False).returncode:
            require(time.monotonic() < deadline, "Core did not recover verified job")
            time.sleep(0.2)
        require(
            expect(200, api(BASE + f"/jobs/{job['id']}", cookie=cookie)) == job,
            "stored verified job did not survive Core restart",
        )
        require(
            expect(202, api(BASE + "/jobs", "POST", queue, cookie, csrf)) == job,
            "post-restart replay changed job",
        )
        require(
            mounted_snapshot(facts, uuid) == snapshot
            and durable_result(
                job, proposal, snapshot, result["binary_sha256"]["limeos-executor"]
            )
            == result["durable"],
            "Core restart repeated effect or changed verified facts",
        )
        preserved_authority(result["authority_before"], authority_snapshot(job["id"]))
        result["data_after"] = {
            name: unchanged_file(
                (TARGET if name == "backup" else Path("/mnt") / name) / "sentinel"
            )
            for name in result["data_before"]
        }
        require(
            result["data_after"] == result["data_before"],
            "sentinel data/ownership changed",
        )
        require(
            digest(Path("/etc/fstab")) == result["fstab_before"],
            "nonpersistent operation changed fstab",
        )
        require(
            {str(path): digest(path) for path in [*CONFIGURATION.values(), POLICY]}
            == result["configuration_sha256"],
            "operation changed protected configuration",
        )
        result.update(
            summary="pass",
            stage="complete",
            finished=time.time(),
            checks=[
                "real managed catalog/plan/digest approval/queue",
                "genuine separate ext4 disk with rw,nosuid,nodev",
                "root Reaped/Verified plus independent Core proof and complete released claims",
                "filesystem-root UID1000 differs from prepared underlay and remains unchanged",
                "same-key replay and verified Core restart never remount",
                "fstab/data/ownership/configuration and prior authority preserved",
            ],
        )
    except Exception as error:  # noqa: BLE001 - Keep the primary failure fatal after bounded cleanup.
        failure = error
        result.update(summary="fail", finished=time.time(), error=str(error)[:2000])
        if evidence is not None:
            try:
                result["unit_diagnostics"] = run(
                    "journalctl",
                    "-u",
                    UNIT,
                    "--no-pager",
                    "-n",
                    "60",
                    timeout=10,
                    check=False,
                ).stdout[-16000:]
            except Exception as diagnostic_error:  # noqa: BLE001 - Diagnostics never replace the retained primary failure.
                result["diagnostic_error"] = str(diagnostic_error)[:1000]
    finally:
        if started:
            # Stopping a unit does not prove helper death, release claims or
            # authorize unmount/repair. The runner disposes of the whole VM.
            try:
                stopped = run("systemctl", "stop", UNIT, timeout=15, check=False)
                result["teardown"] = {
                    "unit_stop_returncode": stopped.returncode,
                    "mount_left_for_vm_disposal": True,
                }
                require(
                    stopped.returncode == 0,
                    "mount unit stop failed; VM disposal still required",
                )
            except Exception as stop_error:  # noqa: BLE001 - Teardown uncertainty remains a fatal qualification failure.
                failure = failure or stop_error
                result.update(summary="fail", error=str(stop_error)[:2000])
    if evidence is not None:
        try:
            if failure is None:
                complete = dict(evidence)
                complete["prior_approved_scope"] = {
                    key: evidence.get(key) for key in ("scope", "limitations")
                }
                complete["scope"] = (
                    "approved target suite plus one real API/Core/root-service nonpersistent ext4 mount, verified replay and same-Core restart"
                )
                complete["limitations"] = LIMITATIONS
                complete["passed"] = [
                    *evidence["passed"],
                    "A1 genuine separate-disk ext4 mount via approved API/Core/root service, independent proof, same-key replay and verified Core restart",
                ]
                publish(output, complete, result)
            else:
                publish(output, evidence, result)
        except Exception as publication_error:  # noqa: BLE001 - A failed publication remains fatal through fallback.
            failure = failure or publication_error
            result.update(
                summary="fail", publication_error=str(publication_error)[:2000]
            )
            try:
                # This fallback retains the prior suite without appending a
                # positive A1 check. It does not assume a failed write worked.
                publish(output, evidence, parse_json(failure_packet(result)))
            except Exception as fallback_error:  # noqa: BLE001 - Retain the fatal failure even if its diagnostic write fails.
                result["fallback_publication_error"] = str(fallback_error)[:1000]
    if failure is not None:
        print("A1_FAILURE " + failure_packet(result), flush=True)
        raise RuntimeError(
            "A1 qualification failed; preserve uncertainty and do not retry"
        ) from failure
    print(
        "PASS A1 real approved ext4 mount and replay; broader gates remain open",
        flush=True,
    )
    return result
