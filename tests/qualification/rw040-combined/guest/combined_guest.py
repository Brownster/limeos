#!/usr/bin/env python3
"""RW-040 combined qualification inside one disposable AMD64 guest.

Started once by cloud-init as guest root from the read-only inputs disk; there
is no login, SSH or host directory. Every effect (packages, disks, users,
Docker daemon, containers, tasks) is inside this guest, which powers off when
done. Results are written as one tar stream to the dedicated results disk.

Without a supplied integrator probe, every probe case is reported as blocked:
fixtures, independent ground truth, confinement equivalence, environment
baselines and the harness self-check still run and are labelled as such.
Debian 12's python3 (3.11) and standard tools only.
"""

import argparse
import hashlib
import http.client
import io
import json
import os
import shutil
import socket
import struct
import subprocess
import sys
import tarfile
import threading
import time
import traceback
from pathlib import Path

RESULTS = Path("/qual/results")
STARTED = time.monotonic()
LOG: list[str] = []
COMMANDS: list[dict] = []
READER_UNIT = "limeos-storage-reader.service"
# [Service] keys that configure the service lifecycle, not its confinement.
LIFECYCLE_KEYS = {
    "Type",
    "ExecStart",
    "RuntimeDirectory",
    "RuntimeDirectoryMode",
    "Restart",
    "RestartSec",
    "TimeoutStartSec",
    "TimeoutStopSec",
}
# Keys the reproduced confinement must carry over exactly, by `systemctl show` name.
SHOW_NAMES = {
    "User": ["User"],
    "Group": ["Group"],
    "UMask": ["UMask"],
    "LimitCORE": ["LimitCORE", "LimitCORESoft"],
    "MemorySwapMax": ["MemorySwapMax"],
    "NoNewPrivileges": ["NoNewPrivileges"],
    "PrivateMounts": ["PrivateMounts"],
    "PrivateDevices": ["PrivateDevices"],
    "RestrictSUIDSGID": ["RestrictSUIDSGID"],
    "RestrictRealtime": ["RestrictRealtime"],
    "LockPersonality": ["LockPersonality"],
    "MemoryDenyWriteExecute": ["MemoryDenyWriteExecute"],
    "RestrictAddressFamilies": ["RestrictAddressFamilies"],
    "CapabilityBoundingSet": ["CapabilityBoundingSet"],
    "AmbientCapabilities": ["AmbientCapabilities"],
    "SystemCallArchitectures": ["SystemCallArchitectures"],
    "SystemCallFilter": ["SystemCallFilter"],
    "LimitNOFILE": ["LimitNOFILE", "LimitNOFILESoft"],
    "TasksMax": ["TasksMax"],
    "MemoryMax": ["MemoryMax"],
    "CPUQuota": ["CPUQuotaPerSecUSec"],
}
# Sandbox properties not set by the unit; transient defaults must equal its defaults.
SANDBOX_DEFAULTS = [
    "ProtectSystem",
    "ProtectHome",
    "ProtectKernelTunables",
    "ProtectKernelModules",
    "ProtectKernelLogs",
    "ProtectControlGroups",
    "ProtectClock",
    "ProtectHostname",
    "ProtectProc",
    "ProcSubset",
    "PrivateTmp",
    "PrivateNetwork",
    "PrivateUsers",
    "PrivateIPC",
    "RestrictNamespaces",
    "DynamicUser",
    "SecureBits",
    "SupplementaryGroups",
    "CapabilityBoundingSet",
    "DevicePolicy",
    "DeviceAllow",
    "IPAddressDeny",
    "IPAddressAllow",
    "Delegate",
    "MemoryHigh",
    "MemoryLow",
    "MemoryMin",
    "LimitNPROC",
    "LimitMEMLOCK",
    "LimitAS",
    "KeyringMode",
    "SystemCallErrorNumber",
    "RootDirectory",
    "RootImage",
    "ReadOnlyPaths",
    "InaccessiblePaths",
    "ReadWritePaths",
    "NetworkNamespacePath",
    "IOSchedulingClass",
    "Nice",
]


def log(message: str) -> None:
    line = f"[rw040 +{time.monotonic() - STARTED:7.1f}s] {message}"
    print(line, flush=True)
    LOG.append(line)


def run(
    *argv: str,
    check: bool = True,
    timeout: float = 600,
    input: str | None = None,
    record: bool = True,
) -> subprocess.CompletedProcess:
    started = time.monotonic()
    done = subprocess.run(
        argv, capture_output=True, text=True, timeout=timeout, input=input, check=False
    )
    if record:
        COMMANDS.append(
            {
                "argv": list(argv),
                "exit": done.returncode,
                "seconds": round(time.monotonic() - started, 3),
                "stdout_tail": done.stdout[-600:],
                "stderr_tail": done.stderr[-600:],
            }
        )
    if check and done.returncode:
        raise RuntimeError(f"{argv[0]} exited {done.returncode}: {done.stderr[-400:]}")
    return done


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def save(name: str, value) -> None:
    path = RESULTS / name
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=True, default=str) + "\n")


def meminfo() -> dict:
    fields = {}
    for line in Path("/proc/meminfo").read_text().splitlines():
        key, value = line.split(":", 1)
        if key in ("MemTotal", "MemAvailable", "SwapTotal", "SwapFree"):
            fields[key] = int(value.split()[0])
    return fields


class UnixHTTP(http.client.HTTPConnection):
    def __init__(self, path: str, timeout: float = 10):
        super().__init__("docker", timeout=timeout)
        self.socket_path = path

    def connect(self):
        sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        sock.settimeout(self.timeout)
        sock.connect(self.socket_path)
        self.sock = sock


def engine(url: str, path: str = "/run/docker.sock"):
    connection = UnixHTTP(path)
    try:
        connection.request("GET", url)
        response = connection.getresponse()
        body = response.read()
        if response.status != 200:
            raise RuntimeError(f"GET {url}: {response.status}")
        return (
            json.loads(body) if body.strip().startswith((b"{", b"[")) else body.decode()
        )
    finally:
        connection.close()


def peer(path: str) -> dict:
    sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    try:
        sock.connect(path)
        pid, uid, gid = struct.unpack(
            "3i", sock.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, 12)
        )
    finally:
        sock.close()
    comm = Path(f"/proc/{pid}/comm").read_text().strip() if pid > 0 else None
    return {"pid": pid, "uid": uid, "gid": gid, "comm": comm}


# ---- stages -------------------------------------------------------------------------


def stage_platform(ctx):
    facts = {
        "uname": os.uname()._asdict()
        if hasattr(os.uname(), "_asdict")
        else list(os.uname()),
        "page_size": os.sysconf("SC_PAGE_SIZE"),
        "cpus": os.cpu_count(),
        "meminfo": meminfo(),
        "os_release": Path("/etc/os-release").read_text(),
        "kernel_cmdline": Path("/proc/cmdline").read_text().strip(),
        "virtualization": run("systemd-detect-virt", check=False).stdout.strip(),
        "cpu_model": next(
            (
                l.split(":", 1)[1].strip()
                for l in Path("/proc/cpuinfo").read_text().splitlines()
                if l.startswith("model name")
            ),
            None,
        ),
        "systemd": run("systemctl", "--version").stdout.splitlines()[0],
        "python": sys.version,
        "ptrace_scope": Path("/proc/sys/kernel/yama/ptrace_scope").read_text().strip()
        if Path("/proc/sys/kernel/yama/ptrace_scope").exists()
        else None,
        "suid_dumpable": Path("/proc/sys/fs/suid_dumpable").read_text().strip(),
        "inputs_manifest": json.loads((ctx.inputs / "manifest.json").read_text()),
        "cloud_init_schema": run(
            "cloud-init", "schema", "--system", check=False
        ).stdout[-2000:]
        + run("cloud-init", "schema", "--system", check=False, record=False).stderr[
            -2000:
        ],
    }
    ctx.platform = facts
    save("platform.json", facts)


def stage_install(ctx):
    manifest = json.loads((ctx.inputs / "manifest.json").read_text())
    verified = {}
    for entry in manifest["packages"]:
        path = ctx.inputs / "packages" / entry["file"]
        actual = sha256(path)
        if actual != entry["sha256"]:
            raise RuntimeError(f"{entry['file']}: package hash mismatch in guest")
        verified[entry["file"]] = actual
    apt = [
        "env",
        "DEBIAN_FRONTEND=noninteractive",
        "apt-get",
        "-o",
        "DPkg::Lock::Timeout=300",
        "-q",
    ]
    run(*apt, "update", timeout=900)
    run(
        *apt,
        "install",
        "-y",
        "--no-install-recommends",
        "docker.io",
        "busybox-static",
        "xfsprogs",
        "btrfs-progs",
        timeout=1500,
    )
    package = ctx.inputs / "packages" / manifest["install_package"]
    run(*apt, "install", "-y", "--no-install-recommends", str(package), timeout=900)
    versions = run(
        "dpkg-query",
        "-W",
        "-f",
        "${Package} ${Version} ${Architecture}\n",
        "docker.io",
        "containerd",
        "runc",
        "busybox-static",
        "limeos",
        "systemd",
        "linux-image-*",
        check=False,
    ).stdout
    ctx.packages = {
        "verified_inputs": verified,
        "installed": versions.splitlines(),
        "installed_package": manifest["install_package"],
    }
    save("packages.json", ctx.packages)


def stage_service(ctx):
    unit = Path("/lib/systemd/system") / READER_UNIT
    if not unit.exists():
        unit = Path("/usr/lib/systemd/system") / READER_UNIT
    text = unit.read_text()
    binaries = {
        p.name: sha256(p)
        for p in sorted(Path("/usr/lib/limeos").iterdir())
        if p.is_file()
    }
    expected = json.loads((ctx.inputs / "manifest.json").read_text())[
        "expected_binaries"
    ]
    mismatched = {k: v for k, v in expected.items() if binaries.get(k) != v}
    if mismatched:
        raise RuntimeError(
            f"installed binaries differ from the CI record: {sorted(mismatched)}"
        )
    properties, unknown = confinement_properties(text)
    if unknown:
        raise RuntimeError(
            f"unit has confinement keys the harness does not map: {unknown}"
        )
    ctx.unit = {
        "path": str(unit),
        "sha256": hashlib.sha256(text.encode()).hexdigest(),
        "text": text,
    }
    ctx.confinement = properties
    state = run(
        "systemctl", "show", READER_UNIT, "-p", "ActiveState,UnitFileState,LoadState"
    ).stdout
    save(
        "service.json",
        {
            "unit": ctx.unit,
            "confinement_properties": properties,
            "binaries_sha256": binaries,
            "reader_state": state.splitlines(),
            "group": run("getent", "group", "limeos-host-access").stdout.strip(),
        },
    )


def confinement_properties(text: str) -> tuple[list[tuple[str, str]], list[str]]:
    """Every non-lifecycle [Service] setting, in order, plus any the harness cannot compare."""
    properties, unknown, section = [], [], None
    for raw in text.splitlines():
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        if line.startswith("["):
            section = line
            continue
        if section != "[Service]":
            continue
        key, value = line.split("=", 1)
        if key in LIFECYCLE_KEYS:
            continue
        if key not in SHOW_NAMES:
            unknown.append(key)
        properties.append((key, value))
    return properties, unknown


def confined(unit: str, argv: list[str], ctx) -> list[str]:
    return [
        "systemd-run",
        "--wait",
        "--pipe",
        "--quiet",
        "--collect",
        f"--unit={unit}",
        "--service-type=exec",
        *[f"--property={k}={v}" for k, v in ctx.confinement],
        *argv,
    ]


def show(unit: str, names: list[str]) -> dict:
    output = run("systemctl", "show", unit, "-p", ",".join(names)).stdout
    return dict(line.split("=", 1) for line in output.splitlines() if "=" in line)


def stage_confinement(ctx):
    names = sorted(
        {n for k, _ in ctx.confinement for n in SHOW_NAMES[k]} | set(SANDBOX_DEFAULTS)
    )
    unit = "rw040-confinement-check"
    run(
        "systemd-run",
        f"--unit={unit}",
        "--service-type=exec",
        "--collect",
        *[f"--property={k}={v}" for k, v in ctx.confinement],
        "/bin/sleep",
        "60",
    )
    try:
        transient = show(f"{unit}.service", names)
        installed = show(READER_UNIT, names)
    finally:
        run("systemctl", "stop", f"{unit}.service", check=False)
    differences = {
        n: {"installed": installed.get(n), "transient": transient.get(n)}
        for n in names
        if installed.get(n) != transient.get(n)
    }
    result = {
        "compared_properties": names,
        "installed": installed,
        "transient": transient,
        "differences": differences,
        "equivalent": not differences,
    }
    save("confinement-equivalence.json", result)
    if differences:
        raise RuntimeError(f"reproduced confinement differs: {sorted(differences)}")


def stage_image(ctx):
    root = Path("/qual/image-root")
    (root / "bin").mkdir(parents=True)
    (root / "newroot/bin").mkdir(parents=True)
    busybox = Path("/bin/busybox")
    applets = run(str(busybox), "--list").stdout.split()
    needed = ["sh", "sleep", "chroot", "unshare", "cat", "test", "touch"]
    missing = [a for a in needed if a not in applets]
    for target in (root / "bin/busybox", root / "newroot/bin/busybox"):
        shutil.copy2(busybox, target)
        target.chmod(0o755)
    shutil.copy2(busybox, root / "bin/busybox-suid")
    os.chmod(root / "bin/busybox-suid", 0o4755)
    for applet in needed:
        if applet in applets:
            (root / "bin" / applet).symlink_to("busybox")
    (root / "newroot/bin/sleep").symlink_to("busybox")
    archive = Path("/qual/image.tar")
    with tarfile.open(archive, "w") as tar:
        tar.add(root, arcname=".")
    run("docker", "import", str(archive), ctx.layout["image_name"])
    image = engine(f"/images/{ctx.layout['image_name']}/json")
    ctx.image = {
        "busybox_sha256": sha256(busybox),
        "applets_missing": missing,
        "image_id": image["Id"],
        "tar_sha256": sha256(archive),
    }
    save("image.json", ctx.image)


def engine_facts() -> dict:
    run_dir = Path("/var/run")
    sock = Path("/run/docker.sock")
    stat = sock.stat()
    return {
        "var_run": {
            "is_symlink": run_dir.is_symlink(),
            "target": os.readlink(run_dir) if run_dir.is_symlink() else None,
        },
        "socket": {
            "path": str(sock),
            "mode": oct(stat.st_mode),
            "uid": stat.st_uid,
            "gid": stat.st_gid,
            "dev": stat.st_dev,
            "ino": stat.st_ino,
            "nlink": stat.st_nlink,
        },
        "peer_canonical": peer("/run/docker.sock"),
        "activation": show(
            "docker.socket",
            [
                "ActiveState",
                "Listen",
                "Accept",
                "SocketUser",
                "SocketGroup",
                "SocketMode",
            ],
        ),
        "daemon": show(
            "docker.service",
            [
                "ActiveState",
                "MainPID",
                "TriggeredBy",
                "ExecMainStartTimestampMonotonic",
            ],
        ),
        "version": engine("/version"),
        "info": {
            k: v
            for k, v in engine("/info").items()
            if k
            in (
                "ID",
                "Name",
                "DockerRootDir",
                "Driver",
                "Containers",
                "ContainersRunning",
                "ContainersStopped",
                "ServerVersion",
                "CgroupVersion",
                "CgroupDriver",
                "SecurityOptions",
            )
        },
    }


def membership() -> dict:
    listed = engine("/containers/json?all=1")
    members = []
    for item in sorted(listed, key=lambda c: c["Id"]):
        inspected = engine(f"/containers/{item['Id']}/json")
        state = inspected["State"]
        members.append(
            {
                "id": inspected["Id"],
                "name": inspected["Name"].lstrip("/"),
                "image": inspected["Image"],
                "running": state["Running"],
                "status": state["Status"],
                "pid": state["Pid"],
                "started_at": state["StartedAt"],
                "user": inspected["Config"].get("User"),
                "mounts": inspected["Mounts"],
                "inspect_sha256": hashlib.sha256(
                    json.dumps(inspected, sort_keys=True).encode()
                ).hexdigest(),
            }
        )
    return {
        "issued_wall": time.time(),
        "issued_monotonic": time.monotonic(),
        "count": len(members),
        "running": sum(m["running"] for m in members),
        "members": members,
    }


def stage_empty(ctx):
    facts = engine_facts()
    members = membership()
    if members["count"]:
        raise RuntimeError("Engine is not empty before fixtures")
    save("ground-truth/engine.json", facts)
    save("ground-truth/membership-empty.json", members)


def device(serial: str) -> str:
    return os.path.realpath(f"/dev/disk/by-id/virtio-{serial}")


def stage_layout(ctx):
    for disk in ctx.layout["disks"]:
        dev = device(disk["serial"])
        if disk["filesystem"] == "ext4":
            run("mkfs.ext4", "-q", "-F", "-U", disk["uuid"], "-L", disk["label"], dev)
        elif disk["filesystem"] == "xfs":
            run(
                "mkfs.xfs",
                "-q",
                "-f",
                "-m",
                f"uuid={disk['uuid']}",
                "-L",
                disk["label"],
                dev,
            )
        elif disk["filesystem"] == "btrfs":
            run("mkfs.btrfs", "-q", "-f", "-U", disk["uuid"], "-L", disk["label"], dev)
    mounted = {}
    for disk in sorted(
        (d for d in ctx.layout["disks"] if d["mount"]),
        key=lambda d: d["mount"].count("/"),
    ):
        Path(disk["mount"]).mkdir(parents=True, exist_ok=True)
        run("mount", device(disk["serial"]), disk["mount"])
        mounted[disk["mount"]] = disk["serial"]
        for directory in ctx.layout["directories"]:
            if directory.startswith(disk["mount"] + "/"):
                Path(directory).mkdir(parents=True, exist_ok=True)
    for directory in ctx.layout["directories"]:
        Path(directory).mkdir(parents=True, exist_ok=True)
    Path("/srv/rw040/main/config/app.conf").write_text("setting = fixture\n")
    Path("/srv/rw040/main/media/item.bin").write_bytes(b"fixture media\n")
    for item in ctx.layout["tmpfs"]:
        run(
            "mount",
            "-t",
            "tmpfs",
            "-o",
            f"size={item['size']},mode=0755",
            "tmpfs",
            item["mount"],
        )
    for bind in ctx.layout["binds"]:
        run("mount", "--bind", bind["source"], bind["target"])
    for link in ctx.layout["symlinks"]:
        Path(link["path"]).symlink_to(link["target"])
    for user in ctx.layout["users"]:
        run(
            "useradd",
            "-u",
            str(user["uid"]),
            "-M",
            "-s",
            "/usr/sbin/nologin",
            user["name"],
        )
    for volume in ctx.layout["volumes"]:
        run("docker", "volume", "create", volume)
    ctx.tasks = {}
    for task in ctx.layout["host_tasks"]:
        if not task["dumpable"]:
            argv = [
                "python3",
                "-c",
                "import ctypes,time; ctypes.CDLL(None).prctl(4,0,0,0,0); time.sleep(36000)",
            ]
        elif task.get("capabilities") == "none":
            argv = [
                "setpriv",
                "--reuid=0",
                "--regid=0",
                "--clear-groups",
                "--inh-caps=-all",
                "--ambient-caps=-all",
                "--bounding-set=-all",
                "sleep",
                "36000",
            ]
        else:
            argv = [
                "setpriv",
                f"--reuid={task['uid']}",
                f"--regid={task['uid']}",
                "--clear-groups",
                "sleep",
                "36000",
            ]
        process = subprocess.Popen(
            argv,
            start_new_session=True,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        ctx.tasks[task["name"]] = process.pid
    time.sleep(0.5)
    save(
        "fixtures/layout-applied.json",
        {
            "mounted": mounted,
            "tasks": ctx.tasks,
            "findmnt": json.loads(
                run(
                    "findmnt",
                    "-J",
                    "-o",
                    "ID,PARENT,TARGET,SOURCE,FSTYPE,MAJ:MIN,FSROOT,OPTIONS,PROPAGATION,UUID",
                ).stdout
            ),
        },
    )


def container_argv(spec: dict, image: str) -> list[str]:
    argv = ["docker", "run", "-d", "--name", spec["name"], "--network", "none"]
    if spec.get("user"):
        argv += ["--user", spec["user"]]
    for cap in spec.get("cap_add", []):
        argv += ["--cap-add", cap]
    for option in spec.get("security_opt", []):
        argv += ["--security-opt", option]
    for mount in spec["mounts"]:
        option = (
            f"type={mount['type']},source={mount['source']},target={mount['target']}"
        )
        if mount.get("read_only"):
            option += ",readonly"
        argv += ["--mount", option]
    command = {
        None: ["/bin/sleep", "36000"],
        "nondumpable": ["/bin/busybox-suid", "sleep", "36000"],
        "root-switch": [
            "/bin/sh",
            "-c",
            (
                "while [ ! -e /trigger/root-switch ]; do sleep 1; done; "
                "exec chroot /newroot /bin/sleep 36000"
            ),
        ],
        "namespace-switch": [
            "/bin/sh",
            "-c",
            (
                "while [ ! -e /trigger/namespace-switch ]; do sleep 1; done; "
                "exec unshare -m /bin/sleep 36000"
            ),
        ],
    }[spec.get("command")]
    return [*argv, image, *command]


def start(spec: dict, image: str) -> None:
    run(*container_argv(spec, image))
    if spec.get("state") == "stopped":
        run("docker", "stop", "-t", "1", spec["name"])


def stage_containers(ctx):
    for spec in [*ctx.layout["containers"], *ctx.layout["transition_containers"]]:
        start(spec, ctx.layout["image_name"])
    time.sleep(1)
    save("ground-truth/membership-complete.json", membership())


def process_facts(pid: int) -> dict:
    base = Path(f"/proc/{pid}")
    status = dict(
        l.split(":", 1) for l in (base / "status").read_text().splitlines() if ":" in l
    )
    stat_fields = (base / "stat").read_text().rsplit(")", 1)[1].split()
    mountinfo = (base / "mountinfo").read_bytes()
    root = os.stat(base / "root/")
    return {
        "pid": pid,
        "uid": status["Uid"].split(),
        "gid": status["Gid"].split(),
        "nspid": status.get("NSpid", "").split(),
        "start_ticks": int(stat_fields[19]),
        "proc_owner_uid": (base / "status").stat().st_uid,
        "nondumpable_indicator": (base / "status").stat().st_uid
        != int(status["Uid"].split()[1]),
        "ns_mnt": os.readlink(base / "ns/mnt"),
        "ns_pid": os.readlink(base / "ns/pid"),
        "root_link": os.readlink(base / "root"),
        "root_dev": root.st_dev,
        "root_ino": root.st_ino,
        "mountinfo_sha256": hashlib.sha256(mountinfo).hexdigest(),
        "mountinfo_rows": mountinfo.count(b"\n"),
    }


def source_facts(path: str) -> dict:
    target = Path(path)
    resolved = os.path.realpath(path)
    stat = os.stat(resolved)
    found = json.loads(
        run(
            "findmnt",
            "-J",
            "-T",
            resolved,
            "-o",
            "ID,TARGET,SOURCE,FSTYPE,MAJ:MIN,FSROOT,UUID",
        ).stdout
    )["filesystems"][0]
    return {
        "declared": path,
        "is_symlink": target.is_symlink(),
        "resolved": resolved,
        "dev": stat.st_dev,
        "ino": stat.st_ino,
        "kind": "directory" if os.path.isdir(resolved) else "file",
        "mount": found,
    }


def stage_ground_truth(ctx):
    members = membership()
    processes, sources = {}, {}
    for member in members["members"]:
        if member["running"]:
            processes[member["name"]] = process_facts(member["pid"])
        for mount in member["mounts"]:
            path = mount["Source"]
            if path and path not in sources:
                sources[path] = source_facts(path)
    tasks = {name: process_facts(pid) for name, pid in ctx.tasks.items()}
    blocks = json.loads(
        run(
            "lsblk",
            "-J",
            "-o",
            "NAME,KNAME,MAJ:MIN,TYPE,SIZE,SERIAL,FSTYPE,UUID,LABEL,MOUNTPOINTS",
        ).stdout
    )
    host = Path("/proc/self/mountinfo").read_bytes()
    ctx.targets = {
        **{f"container:{n}": p["pid"] for n, p in processes.items()},
        **{f"task:{n}": p["pid"] for n, p in tasks.items()},
    }
    save(
        "ground-truth/complete.json",
        {
            "membership": members,
            "processes": processes,
            "host_tasks": tasks,
            "sources": sources,
            "blocks": blocks,
            "host_mountinfo_sha256": hashlib.sha256(host).hexdigest(),
            "boot_id": Path("/proc/sys/kernel/random/boot_id").read_text().strip(),
        },
    )


def stage_environment(ctx):
    probe = str(ctx.inputs / "guest/env_probe.py")
    argv = ["/usr/bin/python3", probe, "--targets", json.dumps(ctx.targets)]
    results = {}
    unrestricted = run(*argv, check=False)
    results["unrestricted_root"] = {
        "exit": unrestricted.returncode,
        "stderr": unrestricted.stderr[-2000:],
        "report": json.loads(unrestricted.stdout)
        if unrestricted.returncode == 0
        else None,
    }
    variants = {
        "reproduced_confinement": ctx.confinement,
        # Diagnostics vary one existing setting to attribute a refusal. They grant
        # nothing and are never qualification modes.
        "diagnostic_without_RestrictSUIDSGID": [
            p for p in ctx.confinement if p[0] != "RestrictSUIDSGID"
        ],
        "diagnostic_without_Group": [p for p in ctx.confinement if p[0] != "Group"],
    }
    for label, properties in variants.items():
        command = [
            "systemd-run",
            "--wait",
            "--pipe",
            "--quiet",
            "--collect",
            f"--unit=rw040-env-{label.replace('_', '-')}",
            "--service-type=exec",
            *[f"--property={k}={v}" for k, v in properties],
            *argv,
        ]
        limited = run(*command, check=False)
        results[label] = {
            "exit": limited.returncode,
            "stderr": limited.stderr[-2000:],
            "removed": sorted(
                {k for k, _ in ctx.confinement} - {k for k, _ in properties}
            ),
            "report": json.loads(limited.stdout) if limited.returncode == 0 else None,
        }
    results["label"] = (
        "environment baseline with standard interfaces; not RW-040 library results. "
        "diagnostic_* variants remove one existing setting only to attribute a refusal"
    )
    save("environment/baseline.json", results)


class Sampler(threading.Thread):
    """Poll one transient unit's main process and cgroup until it exits."""

    def __init__(self, unit: str):
        super().__init__(daemon=True)
        self.unit = unit
        self.peak = {
            "fds": 0,
            "vm_rss_kib": 0,
            "vm_hwm_kib": 0,
            "pss_kib": 0,
            "cgroup_memory_current": 0,
        }
        self.samples = 0
        self.pid = None
        self.cgroup = None
        self.final = {}
        self.stop = threading.Event()

    def run(self):
        deadline = time.monotonic() + 120
        while not self.stop.is_set() and time.monotonic() < deadline:
            values = (
                show(self.unit, ["MainPID", "ControlGroup"]) if self.pid is None else {}
            )
            if self.pid is None:
                if values.get("MainPID", "0") != "0":
                    self.pid = int(values["MainPID"])
                    self.cgroup = Path("/sys/fs/cgroup") / values[
                        "ControlGroup"
                    ].lstrip("/")
                time.sleep(0.005)
                continue
            try:
                fds = len(os.listdir(f"/proc/{self.pid}/fd"))
                status = dict(
                    l.split(":", 1)
                    for l in Path(f"/proc/{self.pid}/status").read_text().splitlines()
                )
                rollup = Path(f"/proc/{self.pid}/smaps_rollup").read_text()
                pss = next(
                    int(l.split()[1])
                    for l in rollup.splitlines()
                    if l.startswith("Pss:")
                )
                io_text = Path(f"/proc/{self.pid}/io").read_text()
                current = int((self.cgroup / "memory.current").read_text())
                peak_cg = (self.cgroup / "memory.peak").read_text().strip()
            except (OSError, StopIteration, ValueError):
                break
            self.samples += 1
            for key, value in (
                ("fds", fds),
                ("vm_rss_kib", int(status["VmRSS"].split()[0])),
                ("vm_hwm_kib", int(status["VmHWM"].split()[0])),
                ("pss_kib", pss),
                ("cgroup_memory_current", current),
            ):
                self.peak[key] = max(self.peak[key], value)
            self.final = {
                "io": dict(l.split(": ") for l in io_text.splitlines()),
                "cgroup_memory_peak": peak_cg,
            }
            time.sleep(0.005)


def measured(unit: str, argv: list[str], ctx, confine: bool) -> dict:
    """Run under a transient unit (confined or not) while sampling it."""
    if confine:
        command = confined(unit, argv, ctx)
    else:
        command = [
            "systemd-run",
            "--wait",
            "--pipe",
            "--quiet",
            "--collect",
            f"--unit={unit}",
            "--service-type=exec",
            *argv,
        ]
    sampler = Sampler(f"{unit}.service")
    sampler.start()
    started = time.monotonic()
    done = run(*command, check=False, timeout=180)
    elapsed = time.monotonic() - started
    sampler.stop.set()
    sampler.join(5)
    leftover = run(
        "systemctl", "is-active", f"{unit}.service", check=False
    ).stdout.strip()
    lines = [l for l in done.stdout.splitlines() if l.startswith("{")]
    return {
        "argv": argv,
        "confined": confine,
        "exit": done.returncode,
        "seconds": round(elapsed, 3),
        "stdout_last": lines[-1] if lines else None,
        "stdout_lines": len(lines),
        "stderr_tail": done.stderr[-800:],
        "sampler": {
            "samples": sampler.samples,
            "pid": sampler.pid,
            "peak": sampler.peak,
            "final": sampler.final,
        },
        "unit_after": leftover,
    }


def stage_selfcheck(ctx):
    probe = str(ctx.inputs / "guest/selfcheck_probe.py")
    cases = {
        "descriptors": ["--fds", "300"],
        "tasks": ["--tasks", "24"],
        "memory": ["--memory-mib", "96"],
    }
    results = {
        "label": "harness self-check of ceilings and measurement; not a qualification probe"
    }
    for name, extra in cases.items():
        for confine in (False, True):
            unit = f"rw040-selfcheck-{name}-{'confined' if confine else 'root'}"
            results[f"{name}/{'confined' if confine else 'unrestricted'}"] = measured(
                unit, ["/usr/bin/python3", probe, *extra], ctx, confine
            )
    save("selfcheck/ceilings.json", results)


def wait_for(check, seconds: float = 15):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        value = check()
        if value:
            return value
        time.sleep(0.2)
    return None


def pid_of(name: str) -> int:
    return engine(f"/containers/{name}/json")["State"]["Pid"]


def changed_facts(pid: int, key: str, before: dict) -> dict | None:
    """Fresh facts for the same PID once `key` differs from `before`."""
    current = process_facts(pid)
    return current if current[key] != before[key] else None


def transition_root(ctx) -> dict:
    before = pid_of("rw040-trans-root")
    facts = process_facts(before)
    Path("/srv/rw040/main/triggers/root-switch").touch()
    changed = wait_for(lambda: changed_facts(before, "root_ino", facts))
    same = pid_of("rw040-trans-root") == before
    return {
        "pid": before,
        "before": facts,
        "after": changed,
        "same_pid": same,
        "verified": bool(changed) and same,
    }


def transition_namespace(ctx) -> dict:
    before = pid_of("rw040-trans-ns")
    facts = process_facts(before)
    Path("/srv/rw040/main/triggers/namespace-switch").touch()
    changed = wait_for(lambda: changed_facts(before, "ns_mnt", facts))
    same = pid_of("rw040-trans-ns") == before
    return {
        "pid": before,
        "before": facts,
        "after": changed,
        "same_pid": same,
        "verified": bool(changed) and same,
    }


def transition_restart(ctx) -> dict:
    before = pid_of("rw040-run-mountless")
    run("docker", "restart", "-t", "1", "rw040-run-mountless")
    after = pid_of("rw040-run-mountless")
    return {"before": before, "after": after, "verified": after not in (0, before)}


def transition_exit(ctx) -> dict:
    run("docker", "stop", "-t", "1", "rw040-run-volume")
    state = engine("/containers/rw040-run-volume/json")["State"]
    return {
        "pid": state["Pid"],
        "running": state["Running"],
        "verified": state["Pid"] == 0,
    }


def transition_swap(ctx) -> dict:
    swap = Path("/srv/rw040/swap")
    swap_pid = pid_of("rw040-run-swap")
    old = source_facts(str(swap))
    view = Path(f"/proc/{swap_pid}/mountinfo").read_text()
    run("umount", str(swap))
    other = next(d for d in ctx.layout["disks"] if d["label"] == "rw040-swap-b")
    run("mount", device(other["serial"]), str(swap))
    new = source_facts(str(swap))
    return {
        "before": old,
        "after": new,
        "container_mountinfo_unchanged": Path(f"/proc/{swap_pid}/mountinfo").read_text()
        == view,
        "verified": old["mount"]["uuid"] != new["mount"]["uuid"],
    }


def transition_btrfs(ctx) -> dict:
    spec = ctx.layout["unsupported_containers"][0]
    start(spec, ctx.layout["image_name"])
    try:
        facts = source_facts("/srv/rw040/btrfs")
        return {
            "source": facts,
            "running": engine(f"/containers/{spec['name']}/json")["State"]["Running"],
            "verified": facts["mount"]["fstype"] == "btrfs",
        }
    finally:
        run("docker", "rm", "-f", spec["name"])


def stage_transitions(ctx):
    report = {
        "label": "fixture transition verification with independent facts; probe cases still blocked"
    }
    for name, check in (
        ("root_switch", transition_root),
        ("namespace_switch", transition_namespace),
        ("restart", transition_restart),
        ("exit", transition_exit),
        ("swapped_device", transition_swap),
        ("unsupported_btrfs", transition_btrfs),
    ):
        try:
            report[name] = check(ctx)
        except Exception as error:  # noqa: BLE001 - each transition is reported on its own
            report[name] = {
                "verified": False,
                "error": f"{type(error).__name__}: {error}",
            }
        if not report[name].get("verified"):
            for container in ("rw040-trans-root", "rw040-trans-ns"):
                logs = run(
                    "docker",
                    "logs",
                    "--tail",
                    "20",
                    container,
                    check=False,
                    record=False,
                )
                report[name].setdefault("container_logs", {})[container] = (
                    logs.stdout + logs.stderr
                )[-1500:]
    save("transitions/verification.json", report)
    failed = [
        k for k, v in report.items() if isinstance(v, dict) and not v.get("verified")
    ]
    if failed:
        raise RuntimeError(f"transition fixtures not verified: {failed}")


def stage_budget(ctx):
    limit = ctx.manifest["budget_total_containers"]
    report = {
        "requested_total": limit,
        "label": "membership-scale fixture feasibility; probe cases still blocked",
        "memory_before": meminfo(),
    }
    current = membership()["count"]
    created = []
    try:
        index = 0
        while current < limit:
            name = f"rw040-budget-{index:02d}"
            result = run(
                "docker",
                "run",
                "-d",
                "--name",
                name,
                "--network",
                "none",
                ctx.layout["image_name"],
                "/bin/sleep",
                "36000",
                check=False,
            )
            index += 1
            if result.returncode:
                report["failure"] = result.stderr[-400:]
                break
            created.append(name)
            current += 1
        snapshot = membership()
        report.update(
            {
                "reached_total": snapshot["count"],
                "running": snapshot["running"],
                "memory_at_total": meminfo(),
                "membership_sha256": hashlib.sha256(
                    json.dumps(snapshot["members"], sort_keys=True).encode()
                ).hexdigest(),
            }
        )
    finally:
        for name in created:
            run("docker", "rm", "-f", name, check=False, record=False)
    report["removed"] = len(created)
    save("budget/feasibility.json", report)


def stage_cases(ctx):
    cases = json.loads((ctx.inputs / "fixtures/cases.json").read_text())
    probe = ctx.inputs / "probe/manifest.json"
    outcome = []
    for case in cases["cases"]:
        entry = {"id": case["id"], "row": case["row"], "requires": case["requires"]}
        missing = [r for r in case["requires"] if r not in ctx.supplied]
        if missing:
            entry.update(
                {
                    "status": "blocked",
                    "missing": missing,
                    "prepared": case["prepared_by"],
                }
            )
        else:
            entry["status"] = (
                "not-run: supplied probe contract not yet implemented in this harness revision"
            )
        outcome.append(entry)
    save(
        "cases.json",
        {
            "probe_supplied": probe.exists(),
            "supplied": sorted(ctx.supplied),
            "cases": outcome,
        },
    )


STAGES = [
    ("platform", stage_platform, []),
    ("install", stage_install, ["platform"]),
    ("service", stage_service, ["install"]),
    ("confinement", stage_confinement, ["service"]),
    ("image", stage_image, ["install"]),
    ("empty", stage_empty, ["install"]),
    ("layout", stage_layout, ["empty"]),
    ("containers", stage_containers, ["layout", "image"]),
    ("ground-truth", stage_ground_truth, ["containers"]),
    ("environment", stage_environment, ["ground-truth", "confinement"]),
    ("selfcheck", stage_selfcheck, ["confinement"]),
    ("transitions", stage_transitions, ["ground-truth"]),
    ("budget", stage_budget, ["containers"]),
    ("cases", stage_cases, []),
]


class Context:
    pass


def write_results(device: str) -> None:
    buffer = io.BytesIO()
    with tarfile.open(fileobj=buffer, mode="w") as tar:
        for path in sorted(RESULTS.rglob("*")):
            if path.is_file():
                tar.add(path, arcname=str(path.relative_to(RESULTS)))
    data = buffer.getvalue()
    with open(device, "r+b") as stream:
        if len(data) > stream.seek(0, os.SEEK_END):
            raise RuntimeError("results exceed the results disk")
        stream.seek(0)
        stream.write(data)
        stream.flush()
        os.fsync(stream.fileno())


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--inputs", type=Path, required=True)
    parser.add_argument("--results", required=True)
    args = parser.parse_args()
    RESULTS.mkdir(parents=True, exist_ok=True)
    ctx = Context()
    ctx.inputs = args.inputs
    ctx.manifest = json.loads((args.inputs / "manifest.json").read_text())
    ctx.layout = json.loads((args.inputs / "fixtures/layout.json").read_text())
    ctx.supplied = set(ctx.manifest.get("supplied", []))
    budget = time.monotonic() + ctx.manifest["guest_budget_seconds"]
    status: dict[str, dict] = {}
    log(f"RW040-GUEST-START run={ctx.manifest['run_id']}")
    for name, function, needs in STAGES:
        blocked = [n for n in needs if status.get(n, {}).get("result") != "passed"]
        if blocked:
            status[name] = {"result": "skipped", "because": blocked}
            log(f"stage {name}: skipped ({blocked})")
            continue
        if time.monotonic() > budget:
            status[name] = {"result": "skipped", "because": ["guest budget expired"]}
            continue
        started = time.monotonic()
        log(f"stage {name}: start")
        try:
            function(ctx)
            status[name] = {"result": "passed"}
        except Exception as error:  # noqa: BLE001 - every failure is recorded
            status[name] = {
                "result": "failed",
                "error": f"{type(error).__name__}: {error}",
                "traceback": traceback.format_exc()[-3000:],
            }
        status[name]["seconds"] = round(time.monotonic() - started, 2)
        log(f"stage {name}: {status[name]['result']} in {status[name]['seconds']}s")
    save("stages.json", status)
    save("commands.json", COMMANDS)
    (RESULTS / "guest-log.txt").write_text("\n".join(LOG) + "\n")
    save(
        "done.json",
        {
            "run_id": ctx.manifest["run_id"],
            "stages": {k: v["result"] for k, v in status.items()},
            "seconds": round(time.monotonic() - STARTED, 1),
        },
    )
    write_results(args.results)
    log("RW040-GUEST-RESULTS-WRITTEN")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    finally:
        subprocess.run(["systemctl", "poweroff", "--no-block"], check=False)
