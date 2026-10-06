#!/usr/bin/env python3
"""Install native ARM64 packages and qualify service behaviour and footprint.

Runs only inside the disposable guest, as root:
  install_guest.py --repo REPO --expected BUILD_JSON --hashes HASHES --output RESULT

Every check records its raw observation and continues after a failure, so the
report shows exactly which gates passed, failed or were not reached.
"""

import argparse
import hashlib
import http.client
import json
import os
import platform
import re
import sqlite3
import subprocess
import sys
import threading
import time
from pathlib import Path

from qualification import (
    BASE_UNITS,
    BINARIES,
    OPTIONAL_UNITS,
    guest_guard,
    installed_binaries,
    load_build,
    sha,
)

LIMEOS = Path("/usr/lib/limeos")
UNITS = list(BASE_UNITS)
DB = "/var/lib/limeos/core/core.sqlite"
RESULT = {"kind": "install", "checks": [], "observations": {}}


def save():
    RESULT["finished"] = time.time()
    OUTPUT.write_text(json.dumps(RESULT, indent=2, default=str) + "\n")


def run(*args, check=True, input=None, timeout=600):
    result = subprocess.run(
        args, check=False, input=input, text=True, capture_output=True, timeout=timeout
    )
    if check and result.returncode != 0:
        raise RuntimeError(
            f"{args} failed ({result.returncode}): {result.stderr[-3000:]}"
        )
    return result


def check(name):
    def decorator(fn):
        started = time.monotonic()
        try:
            observation = fn()
            RESULT["checks"].append(
                {
                    "name": name,
                    "result": "pass",
                    "seconds": round(time.monotonic() - started, 2),
                    "observation": observation,
                }
            )
            print("PASS " + name, flush=True)
        except Exception as error:  # noqa: BLE001 - record every failure, then continue
            RESULT["checks"].append(
                {
                    "name": name,
                    "result": "fail",
                    "seconds": round(time.monotonic() - started, 2),
                    "error": repr(error)[-4000:],
                }
            )
            print("FAIL " + name + ": " + repr(error)[-500:], flush=True)
        save()
        return fn

    return decorator


def http_request(
    method, path, body=None, cookie=None, csrf=None, port=8003, timeout=15
):
    connection = http.client.HTTPConnection("127.0.0.1", port, timeout=timeout)
    headers = {"Origin": "https://localhost", "Content-Type": "application/json"}
    if cookie:
        headers["Cookie"] = cookie
    if csrf:
        headers["X-CSRF-Token"] = csrf
    connection.request(
        method, path, json.dumps(body) if body is not None else None, headers
    )
    response = connection.getresponse()
    data = response.read()
    connection.close()
    return (
        response.status,
        dict(response.getheaders()),
        (json.loads(data) if data else None),
    )


def login(username, password, retry=True):
    started = time.monotonic()
    code, headers, body = http_request(
        "POST", "/api/v1/auth/login", {"username": username, "password": password}
    )
    elapsed = round((time.monotonic() - started) * 1000, 2)
    if code == 429 and retry:
        # Core allows 10 login attempts per client per 60 s window; wait it out once.
        time.sleep(61)
        return login(username, password, retry=False)
    if code != 200:
        return code, None, None, elapsed
    return code, headers["set-cookie"].split(";", 1)[0], body["csrf_token"], elapsed


def elf_machine(path):
    # Read e_machine from the ELF header directly; the clean guest has no binutils.
    header = Path(path).read_bytes()[:20]
    assert header[:4] == b"\x7fELF", path
    code = int.from_bytes(header[18:20], "little")
    return {183: "AArch64", 62: "X86-64"}.get(code, f"e_machine={code}")


def identity(expected):
    verify = run("dpkg", "--verify", "limeos", check=False)
    lines = [line for line in verify.stdout.splitlines() if line.strip()]
    # Configuration files legitimately change after setup; every other path must match.
    conffiles = [line for line in lines if re.match(r"^\S+\s+c\s", line)]
    assert not [line for line in lines if line not in conffiles], verify.stdout
    installed = installed_binaries(expected)
    pool = {
        p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in REPO.rglob("*.deb")
    }
    for name, digest in pool.items():
        assert expected["packages"][name] == digest, name
    machines = {n: elf_machine(LIMEOS / n) for n in BINARIES}
    assert set(machines.values()) == {"AArch64"}, machines
    with sqlite3.connect(f"file:{DB}?mode=ro", uri=True) as db:
        schema = db.execute("PRAGMA user_version").fetchone()[0]
    assert schema == expected["identity"]["authority_schema"], schema
    return {
        "dpkg_verify_non_conffile_mismatches": 0,
        "changed_conffiles": conffiles,
        "installed_sha256": installed,
        "repository_packages": pool,
        "elf_machine": machines,
        "authority_schema": schema,
    }


def main_pid(unit):
    return int(run("systemctl", "show", unit, "-p", "MainPID", "--value").stdout)


def smaps(pid):
    fields = {}
    for line in Path(f"/proc/{pid}/smaps_rollup").read_text().splitlines()[1:]:
        if ":" in line:
            fields[line.split(":")[0]] = int(line.split()[1])
    return {
        "pss_kib": fields["Pss"],
        "rss_kib": fields["Rss"],
        "swap_kib": fields.get("Swap", 0),
    }


def cgroup(unit):
    path = run(
        "systemctl", "show", unit, "-p", "ControlGroup", "--value"
    ).stdout.strip()
    return Path("/sys/fs/cgroup" + path)


def cpu_usec(unit):
    for line in (cgroup(unit) / "cpu.stat").read_text().splitlines():
        if line.startswith("usage_usec"):
            return int(line.split()[1])
    return 0


def written_bytes(unit):
    total = 0
    stat = cgroup(unit) / "io.stat"
    if stat.exists():
        for line in stat.read_text().splitlines():
            match = re.search(r"wbytes=(\d+)", line)
            if match:
                total += int(match.group(1))
    return total


def vda_sectors_written():
    for line in Path("/proc/diskstats").read_text().splitlines():
        parts = line.split()
        if parts[2] == "vda":
            return int(parts[9])
    return 0


def percentile(values, fraction):
    ordered = sorted(values)
    return ordered[min(len(ordered) - 1, round(fraction * (len(ordered) - 1)))]


def summary(values):
    return {
        "n": len(values),
        "min": min(values),
        "p50": percentile(values, 0.5),
        "p95": percentile(values, 0.95),
        "p99": percentile(values, 0.99),
        "max": max(values),
        "samples": values,
    }


def wait_ready(timeout=5):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if run(str(LIMEOS / "limeosctl"), "status", check=False).returncode == 0:
            return True
        time.sleep(0.005)
    return False


def memory_snapshot():
    services = {unit: smaps(main_pid(unit)) for unit in UNITS if main_pid(unit)}
    docker = {}
    for unit in ["docker", "containerd"]:
        pid = main_pid(unit)
        if pid:
            docker[unit] = smaps(pid)
    shims = [
        p for p in run("pgrep", "-f", "containerd-shim", check=False).stdout.split()
    ]
    workload = [p for p in run("pgrep", "-x", "busybox", check=False).stdout.split()]
    return {
        "limeos_services": services,
        "limeos_sum": {
            k: sum(s[k] for s in services.values())
            for k in ["pss_kib", "rss_kib", "swap_kib"]
        },
        "optional_services": {
            u: smaps(main_pid(u)) for u in OPTIONAL_UNITS if main_pid(u)
        },
        "loadavg": Path("/proc/loadavg").read_text().strip(),
        "docker_engine": docker,
        "containerd_shims": {
            "count": len(shims),
            "pss_kib": sum(smaps(p)["pss_kib"] for p in shims),
        },
        "media_workload": {
            "processes": len(workload),
            "pss_kib": sum(smaps(p)["pss_kib"] for p in workload),
        },
        "password_workers_running": len(
            run("pgrep", "-f", "limeos-password-worker", check=False).stdout.split()
        ),
        "meminfo_available_kib": int(
            re.search(
                r"MemAvailable:\s+(\d+)", Path("/proc/meminfo").read_text()
            ).group(1)
        ),
    }


def window(seconds, label, during=None):
    units = UNITS + ["docker", "containerd"]
    before = {u: (cpu_usec(u), written_bytes(u)) for u in units}
    pids = {u: main_pid(u) for u in UNITS}
    pid_io = {
        u: int(
            re.search(r"write_bytes: (\d+)", Path(f"/proc/{p}/io").read_text()).group(1)
        )
        for u, p in pids.items()
    }
    sectors, wall = vda_sectors_written(), time.monotonic()
    samples = []
    thread = threading.Thread(target=during, args=(seconds,)) if during else None
    if thread:
        thread.start()
    while time.monotonic() - wall < seconds:
        samples.append(
            {
                "elapsed_seconds": time.monotonic() - wall,
                "memory": memory_snapshot(),
                "counters": {
                    u: {
                        "cpu_usec": cpu_usec(u),
                        "cgroup_written_bytes": written_bytes(u),
                    }
                    for u in units
                },
            }
        )
        time.sleep(min(10, max(0, seconds - (time.monotonic() - wall))))
    if thread:
        thread.join(timeout=30)
        assert not thread.is_alive(), "dashboard workload failed to stop"
    elapsed = time.monotonic() - wall
    after = {u: (cpu_usec(u), written_bytes(u)) for u in units}
    per_unit = {}
    for u in units:
        cpu = after[u][0] - before[u][0]
        per_unit[u] = {
            "cpu_seconds": round(cpu / 1e6, 4),
            "cpu_percent_of_one_core": round(100 * cpu / 1e6 / elapsed, 4),
            "cgroup_written_bytes": after[u][1] - before[u][1],
        }
        if u in pids and main_pid(u) == pids[u]:
            now = int(
                re.search(
                    r"write_bytes: (\d+)", Path(f"/proc/{pids[u]}/io").read_text()
                ).group(1)
            )
            per_unit[u]["process_write_bytes"] = now - pid_io[u]
    limeos_cpu = sum(per_unit[u]["cpu_seconds"] for u in UNITS)
    limeos_written = sum(per_unit[u]["cgroup_written_bytes"] for u in UNITS)
    return {
        "raw_samples": samples,
        "counters_before": before,
        "counters_after": after,
        "label": label,
        "seconds": round(elapsed, 1),
        "per_unit": per_unit,
        "limeos_cpu_percent_of_one_core": round(100 * limeos_cpu / elapsed, 4),
        "limeos_written_bytes": limeos_written,
        "limeos_written_bytes_per_day_extrapolated": round(
            limeos_written * 86400 / elapsed
        ),
        "vda_bytes_written_all_processes": (vda_sectors_written() - sectors) * 512,
        "cgroup_io_stat_present": {u: (cgroup(u) / "io.stat").exists() for u in UNITS},
    }


def main():
    global REPO, OUTPUT, VERSION
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", type=Path, required=True)
    parser.add_argument("--expected", type=Path, required=True)
    parser.add_argument("--hashes", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--identity-only", action="store_true")
    args = parser.parse_args()
    guest_guard()
    REPO, OUTPUT = args.repo, args.output
    if OUTPUT.exists():
        parser.error("output exists; select a new result file")
    expected = load_build(args.expected, REPO)
    VERSION = expected["identity"]["package_version"]
    RESULT.update(
        identity=expected["identity"],
        build_result_sha256=sha(args.expected),
        version=VERSION,
        started=time.time(),
    )
    if args.identity_only:
        # Rerun only the identity check against an already installed guest.
        check(
            "package identity: installed files match the package and the native build"
        )(lambda: identity(expected))
        sys.exit(1 if any(c["result"] == "fail" for c in RESULT["checks"]) else 0)
    passwords = json.loads(args.hashes.read_text())
    cpuinfo = Path("/proc/cpuinfo").read_text()
    RESULT["observations"]["platform"] = {
        "uname": " ".join(platform.uname()),
        "virtualization": run("systemd-detect-virt", check=False).stdout.strip(),
        "cpu_implementer": sorted(
            set(re.findall(r"CPU implementer\s*:\s*(\S+)", cpuinfo))
        ),
        "cpu_part": sorted(set(re.findall(r"CPU part\s*:\s*(\S+)", cpuinfo))),
        "cpu_features": sorted(set(re.findall(r"Features\s*:\s*(.*)", cpuinfo)))[:1],
        "nproc": os.cpu_count(),
        "page_size": os.sysconf("SC_PAGE_SIZE"),
        "meminfo_total_kib": int(
            re.search(r"MemTotal:\s+(\d+)", Path("/proc/meminfo").read_text()).group(1)
        ),
        "os_release": Path("/etc/os-release").read_text().splitlines()[:4],
        "block_devices": run("lsblk", "-d", "-o", "NAME,SIZE,ROTA,SERIAL").stdout,
        "loadavg": Path("/proc/loadavg").read_text().strip(),
        "enabled_services": run(
            "systemctl", "list-unit-files", "--state=enabled", "--no-pager"
        ).stdout,
        "workload": {
            "containers": 20,
            "dashboards": 3,
            "synthetic_registered_disks": 0,
        },
    }
    save()

    @check("native aarch64 under KVM")
    def _():
        p = RESULT["observations"]["platform"]
        assert p["virtualization"] == "kvm", p["virtualization"]
        return {
            k: p[k]
            for k in [
                "uname",
                "virtualization",
                "cpu_implementer",
                "cpu_part",
                "page_size",
            ]
        }

    @check("Debian docker.io and a static busybox workload of 20 containers")
    def _():
        run("apt-get", "update", "-qq", timeout=900)
        run(
            "apt-get",
            "install",
            "-y",
            "--no-install-recommends",
            "docker.io",
            "busybox-static",
            "apt-utils",
            timeout=1800,
        )
        import io
        import tarfile

        buffer = io.BytesIO()
        with tarfile.open(fileobj=buffer, mode="w") as archive:
            archive.add("/bin/busybox", arcname="bin/busybox")
        subprocess.run(
            ["docker", "import", "-", "limeos-arm64-fixture:local"],
            input=buffer.getvalue(),
            check=True,
            capture_output=True,
        )
        for index in range(20):
            run(
                "docker",
                "run",
                "-d",
                "--name",
                f"media-{index:02d}",
                "--label",
                "limeos.qualification=arm64",
                "limeos-arm64-fixture:local",
                "/bin/busybox",
                "sleep",
                "86400",
            )
        versions = run(
            "dpkg-query",
            "-W",
            "-f",
            "${Package} ${Version}\n",
            "docker.io",
            "containerd",
            "busybox-static",
        ).stdout
        return {
            "running": len(run("docker", "ps", "-q").stdout.split()),
            "versions": versions,
        }

    @check(f"signed test repository installs limeos {VERSION} for arm64")
    def _():
        run(
            "install",
            "-m",
            "0644",
            str(REPO / "limeos-archive-keyring.gpg"),
            "/usr/share/keyrings/limeos-test.gpg",
        )
        Path("/etc/apt/sources.list.d/limeos-test.list").write_text(
            f"deb [signed-by=/usr/share/keyrings/limeos-test.gpg] file:{REPO} stable main\n"
        )
        run("apt-get", "update", "-qq", timeout=600)
        run("apt-get", "install", "-y", f"limeos={VERSION}", timeout=900)
        identity = run(
            "dpkg-query", "-W", "-f", "${Package} ${Version} ${Architecture}", "limeos"
        ).stdout
        assert identity == f"limeos {VERSION} arm64", identity
        return identity

    @check("package identity: installed files match the package and the native build")
    def _():
        return identity(expected)

    if RESULT["checks"][-1]["result"] != "pass":
        raise SystemExit(
            "installed identity gate failed; footprint measurement refused"
        )

    @check("unit files, enabled state and running services")
    def _():
        files = run(
            "systemctl", "list-unit-files", "limeos*", "--no-legend", "--no-pager"
        ).stdout
        state = {}
        for unit in [
            "limeos-core",
            "limeos-containerd",
            "limeos-storaged",
            "limeos-storage-reader",
            "limeos-storage-ready",
            "limeos-storage-targets",
        ]:
            state[unit] = {
                "enabled": run(
                    "systemctl", "is-enabled", unit, check=False
                ).stdout.strip(),
                "active": run(
                    "systemctl", "is-active", unit, check=False
                ).stdout.strip(),
            }
        for unit in UNITS:
            assert state[unit]["active"] == "active", (unit, state[unit])
        for unit in [*OPTIONAL_UNITS, "limeos-storage-ready"]:
            assert state[unit]["enabled"] == "static", (unit, state[unit])
            assert state[unit]["active"] != "active", (unit, state[unit])
        assert wait_ready(10)
        return {"unit_files": files, "state": state}

    @check("service accounts, capabilities and sandbox properties")
    def _():
        report = {}
        for unit in UNITS:
            properties = run(
                "systemctl",
                "show",
                unit,
                "-p",
                "User",
                "-p",
                "Group",
                "-p",
                "SupplementaryGroups",
                "-p",
                "CapabilityBoundingSet",
                "-p",
                "AmbientCapabilities",
                "-p",
                "NoNewPrivileges",
                "-p",
                "ProtectSystem",
                "-p",
                "MemoryMax",
                "-p",
                "TasksMax",
                "-p",
                "ExecStart",
            ).stdout
            status = Path(f"/proc/{main_pid(unit)}/status").read_text()
            process = {
                k: re.search(rf"^{k}:\s*(.*)$", status, re.MULTILINE).group(1)
                for k in ["Uid", "CapEff", "CapBnd", "CapAmb", "NoNewPrivs"]
            }
            report[unit] = {"properties": properties, "process": process}
            assert (
                process["CapEff"] == "0000000000000000"
                and process["CapAmb"] == "0000000000000000"
            ), (unit, process)
            assert process["NoNewPrivs"] == "1", (unit, process)
        assert report["limeos-core"]["process"]["Uid"].split()[0] != "0"
        assert report["limeos-containerd"]["process"]["Uid"].split()[0] != "0"
        return report

    @check("root-executed paths are root-owned and not writable by other accounts")
    def _():
        checked = set()
        problems = []
        targets = [LIMEOS / name for name in BINARIES]
        for target in list(targets):
            for line in run("ldd", str(target), check=False).stdout.splitlines():
                match = re.search(r"(/\S+)", line)
                if match:
                    targets.append(Path(match.group(1)))
        for target in targets:
            for path in [target.resolve(), *target.resolve().parents]:
                if path in checked:
                    continue
                checked.add(path)
                info = path.stat()
                if info.st_uid != 0 or info.st_mode & 0o022:
                    problems.append(
                        f"{path} uid={info.st_uid} mode={oct(info.st_mode)}"
                    )
        assert not problems, problems
        return {"paths_checked": len(checked)}

    @check("only the container executor process holds Docker socket access")
    def _():
        probe = "import socket,sys\ns=socket.socket(socket.AF_UNIX)\ntry:\n s.connect('/run/docker.sock');print('connected')\nexcept OSError as e:\n print(e.errno)\n"
        docker_gid = int(run("getent", "group", "docker").stdout.split(":")[2])
        groups = {}
        for unit in UNITS:
            status = Path(f"/proc/{main_pid(unit)}/status").read_text()
            groups[unit] = [
                int(g)
                for g in re.search(r"^Groups:\s*(.*)$", status, re.MULTILINE)
                .group(1)
                .split()
            ]
        socket_info = Path("/run/docker.sock").stat()
        outcome = {
            "core_account_connect": run(
                "runuser", "-u", "limeos-core", "--", "python3", "-c", probe
            ).stdout.strip(),
            "docker_gid": docker_gid,
            "process_groups": groups,
            "socket": {
                "uid": socket_info.st_uid,
                "gid": socket_info.st_gid,
                "mode": oct(socket_info.st_mode & 0o777),
            },
        }
        assert outcome["core_account_connect"] == "13", outcome
        assert docker_gid in groups["limeos-containerd"], outcome
        assert (
            docker_gid not in groups["limeos-core"]
            and docker_gid not in groups["limeos-storaged"]
        ), outcome
        return outcome

    state = {}

    @check("no default login; one-use local enrollment and authenticated session")
    def _():
        assert login("admin", "pihealth")[0] == 401
        issued = json.loads(run(str(LIMEOS / "limeosctl"), "bootstrap").stdout)
        state["password"] = (
            "arm64-qualification-" + hashlib.sha256(os.urandom(16)).hexdigest()[:16]
        )
        run(
            str(LIMEOS / "limeosctl"),
            "enroll",
            input=json.dumps(
                {
                    "token": issued["token"],
                    "username": "alice",
                    "password": state["password"],
                }
            ),
        )
        assert run(str(LIMEOS / "limeosctl"), "bootstrap", check=False).returncode != 0
        code, cookie, csrf, elapsed = login("alice", state["password"])
        assert code == 200
        state["cookie"], state["csrf"] = cookie, csrf
        assert http_request("GET", "/api/v1/auth/session", cookie=cookie)[0] == 200
        return {"login_ms": elapsed}

    @check(
        "imported Werkzeug scrypt and PBKDF2-SHA256 hashes verify and upgrade to Argon2id"
    )
    def _():
        run("systemctl", "stop", "limeos-core")
        with sqlite3.connect(DB) as db:
            for i, row in enumerate(passwords):
                db.execute(
                    "INSERT INTO users VALUES(?,?,?, ?,1)",
                    (f"legacy-{i}", f"legacy-{i}", row["hash"], '"administrator"'),
                )
        run("systemctl", "start", "limeos-core")
        assert wait_ready(10)
        timings = {}
        for i, row in enumerate(passwords):
            assert login(f"legacy-{i}", "wrong-password")[0] == 401
            code, _, _, elapsed = login(f"legacy-{i}", row["password"])
            assert code == 200
            with sqlite3.connect(DB) as db:
                stored = db.execute(
                    "SELECT password_hash FROM users WHERE id=?", (f"legacy-{i}",)
                ).fetchone()[0]
            assert stored.startswith("$argon2id$"), stored[:20]
            upgraded_ms = login(f"legacy-{i}", row["password"])[3]
            timings[row["hash"].split("$")[0]] = {
                "first_login_ms": elapsed,
                "argon2id_login_ms": upgraded_ms,
            }
        return timings

    @check("password work runs in a bounded short-lived worker that exits")
    def _():
        samples = {
            "max_concurrent": 0,
            "max_vmhwm_kib": 0,
            "max_rss_kib": 0,
            "max_pss_kib": 0,
            "max_swap_kib": 0,
            "pids": set(),
        }
        stop = threading.Event()

        def sampler():
            while not stop.is_set():
                pids = run(
                    "pgrep",
                    "-f",
                    "^/usr/lib/limeos/limeos-password-worker",
                    check=False,
                ).stdout.split()
                samples["max_concurrent"] = max(samples["max_concurrent"], len(pids))
                for pid in pids:
                    samples["pids"].add(pid)
                    try:
                        status = Path(f"/proc/{pid}/status").read_text()
                        hwm = int(re.search(r"VmHWM:\s+(\d+)", status).group(1))
                        rss = int(re.search(r"VmRSS:\s+(\d+)", status).group(1))
                        samples["max_vmhwm_kib"] = max(samples["max_vmhwm_kib"], hwm)
                        samples["max_rss_kib"] = max(samples["max_rss_kib"], rss)
                        memory = smaps(pid)
                        samples["max_pss_kib"] = max(
                            samples["max_pss_kib"], memory["pss_kib"]
                        )
                        samples["max_swap_kib"] = max(
                            samples["max_swap_kib"], memory["swap_kib"]
                        )
                    except (FileNotFoundError, AttributeError, ProcessLookupError):
                        pass

        thread = threading.Thread(target=sampler)
        thread.start()
        latencies = []
        results = []

        def one():
            code, _, _, elapsed = login("alice", state["password"], retry=False)
            results.append(code)
            latencies.append(elapsed)

        # Restart core so the 10-per-minute login window starts empty: 4 serial + 6 concurrent.
        run("systemctl", "restart", "limeos-core")
        assert wait_ready(10)
        for _ in range(4):
            one()
        burst = [threading.Thread(target=one) for _ in range(6)]
        for t in burst:
            t.start()
        for t in burst:
            t.join()
        time.sleep(0.5)
        stop.set()
        thread.join()
        remaining = run(
            "pgrep", "-f", "^/usr/lib/limeos/limeos-password-worker", check=False
        ).stdout.split()
        assert not remaining, remaining
        assert samples["pids"], "no worker observed"
        assert samples["max_concurrent"] <= 2, samples["max_concurrent"]
        assert set(results) <= {200, 429} and results.count(200) >= 4, results
        core_after = smaps(main_pid("limeos-core"))
        return {
            "logins": len(results),
            "status_codes": {str(c): results.count(c) for c in sorted(set(results))},
            "login_ms": summary(latencies),
            "workers_observed": len(samples["pids"]),
            "max_concurrent_workers": samples["max_concurrent"],
            "max_worker_vmhwm_kib": samples["max_vmhwm_kib"],
            "max_worker_rss_kib": samples["max_rss_kib"],
            "max_worker_pss_kib": samples["max_pss_kib"],
            "max_worker_swap_kib": samples["max_swap_kib"],
            "core_after": core_after,
        }

    @check("sessions survive core and executor restarts")
    def _():
        code, cookie, csrf, _ = login("alice", state["password"])
        assert code == 200
        state["cookie"], state["csrf"] = cookie, csrf
        for unit in ["limeos-core", "limeos-containerd", "limeos-storaged"]:
            run("systemctl", "reset-failed", unit)
            run("systemctl", "restart", unit)
            assert wait_ready(10)
            time.sleep(1)
            status = http_request("GET", "/api/v1/auth/session", cookie=cookie)[0]
            assert status == 200, (unit, status)
            assert http_request("GET", "/api/v1/overview", cookie=cookie)[0] == 200
        return (
            "session valid after restarting core, container executor and host executor"
        )

    @check("core restart readiness distribution (20 restarts)")
    def _():
        values = []
        for _ in range(20):
            run("systemctl", "reset-failed", "limeos-core")
            started = time.monotonic()
            run("systemctl", "restart", "limeos-core")
            assert wait_ready(5)
            values.append(round((time.monotonic() - started) * 1000, 2))
            time.sleep(2.5)  # stay inside the unit's production start-rate limit
        return summary(values)

    @check("full stack cold start readiness (5 starts)")
    def _():
        values = []
        for _ in range(5):
            run("systemctl", "stop", *UNITS)
            for unit in UNITS:
                run("systemctl", "reset-failed", unit)
            started = time.monotonic()
            run("systemctl", "start", *UNITS)
            assert wait_ready(10)
            values.append(round((time.monotonic() - started) * 1000, 2))
            time.sleep(3)
        return summary(values)

    @check("idle footprint after authentication (no subscribers)")
    def _():
        code, cookie, csrf, _ = login("alice", state["password"])
        assert code == 200
        state["cookie"], state["csrf"] = cookie, csrf
        assert http_request("GET", "/api/v1/overview", cookie=cookie)[0] == 200
        time.sleep(60)
        snapshot = memory_snapshot()
        assert snapshot["password_workers_running"] == 0
        return snapshot

    @check("idle CPU and bytes written over 600 seconds, no subscribers")
    def _():
        return window(600, "idle, authenticated, no dashboards")

    def dashboards(seconds):
        stop = threading.Event()
        received = []
        failures = []

        def subscriber():
            connection = http.client.HTTPConnection(
                "127.0.0.1", 8003, timeout=seconds + 30
            )
            connection.request(
                "GET",
                "/api/v1/observations/stream",
                headers={"Cookie": state["cookie"], "Origin": "https://localhost"},
            )
            try:
                response = connection.getresponse()
                assert response.status == 200, response.status
                count = 0
                while not stop.is_set():
                    line = response.fp.readline()
                    if not line:
                        break
                    count += line.startswith(b"data:")
                received.append(count)
            except (
                OSError,
                ValueError,
                http.client.HTTPException,
                AssertionError,
            ) as error:
                failures.append(repr(error))
            finally:
                connection.close()

        threads = [threading.Thread(target=subscriber, daemon=True) for _ in range(3)]
        for t in threads:
            t.start()
        time.sleep(seconds)
        stop.set()
        for thread in threads:
            thread.join(timeout=5)
        state["sse_events"] = received
        state["sse_failures"] = failures

    @check("CPU and bytes written over 300 seconds with 3 open dashboard streams")
    def _():
        measured = window(300, "3 SSE dashboard subscribers", during=dashboards)
        measured["sse_streams_events"] = state.get("sse_events")
        measured["sse_failures"] = state.get("sse_failures")
        assert (
            not measured["sse_failures"] and len(measured["sse_streams_events"]) == 3
        ), measured
        assert all(measured["sse_streams_events"]), measured
        measured["memory_after"] = memory_snapshot()
        return measured

    @check("authenticated read latency, 3 concurrent clients x 200 requests")
    def _():
        paths = [
            "/api/v1/overview",
            "/api/v1/resources",
            "/api/v1/system/history?range=30d",
        ]
        timings = {p: [] for p in paths}
        errors = []

        def client(offset):
            for i in range(200):
                path = paths[(i + offset) % len(paths)]
                started = time.monotonic()
                try:
                    code = http_request("GET", path, cookie=state["cookie"])[0]
                    timings[path].append(round((time.monotonic() - started) * 1000, 3))
                    if code != 200:
                        errors.append((path, code))
                except (OSError, ValueError, http.client.HTTPException) as error:
                    errors.append((path, repr(error)))

        threads = [threading.Thread(target=client, args=(n,)) for n in range(3)]
        started = time.monotonic()
        for t in threads:
            t.start()
        for t in threads:
            t.join()
        total = time.monotonic() - started
        sequential = []
        for _ in range(20):
            began = time.monotonic()
            assert (
                http_request("GET", "/api/v1/overview", cookie=state["cookie"])[0]
                == 200
            )
            sequential.append(round((time.monotonic() - began) * 1000, 3))
        assert not errors, errors[:5]
        assert sum(len(v) for v in timings.values()) == 600, (
            "incomplete latency workload"
        )
        return {
            "per_endpoint_ms": {p: summary(v) for p, v in timings.items()},
            "all_ms": summary([x for v in timings.values() for x in v]),
            "requests_per_second": round(600 / total, 1),
            "sequential_cached_overview_ms": summary(sequential),
            "memory_after": memory_snapshot(),
        }

    @check(
        "shadow installs beside standard, listens separately and refuses write ceilings"
    )
    def _():
        core_pid = main_pid("limeos-core")
        run("apt-get", "install", "-y", f"limeos-shadow={VERSION}", timeout=900)
        installed_binaries(expected, Path("/usr/lib/limeos-shadow"))
        assert not Path(
            "/lib/systemd/system/limeos-shadow-storage-targets.service"
        ).exists()
        units = run(
            "systemctl",
            "list-units",
            "limeos-shadow*",
            "--no-legend",
            "--plain",
            "--no-pager",
        ).stdout
        exec_start = run(
            "systemctl",
            "show",
            "limeos-shadow-containerd",
            "-p",
            "ExecStart",
            "--value",
        ).stdout
        assert "read-only" in exec_start, exec_start
        assert http_request("GET", "/api/v1/health", port=8004)[0] == 200
        policy = Path("/etc/limeos-shadow/system-policy/container.json")
        original = policy.read_text()
        refused = {}
        try:
            for action in ["restart", "start", "stop"]:
                value = {
                    **json.loads(original),
                    "allow_restart": False,
                    "allow_start": False,
                    "allow_stop": False,
                    "allow_" + action: True,
                    "managed_containers": ["0" * 64],
                }
                policy.write_text(json.dumps(value))
                run("systemctl", "reset-failed", "limeos-shadow-containerd")
                run("systemctl", "restart", "limeos-shadow-containerd", check=False)
                deadline = time.monotonic() + 10
                result = ""
                while time.monotonic() < deadline:
                    result = run(
                        "systemctl",
                        "show",
                        "limeos-shadow-containerd",
                        "-p",
                        "Result",
                        "--value",
                    ).stdout.strip()
                    active = run(
                        "systemctl",
                        "is-active",
                        "limeos-shadow-containerd",
                        check=False,
                    ).stdout.strip()
                    if result == "exit-code" and active != "active":
                        break
                    time.sleep(0.2)
                refused[action] = result
                assert result == "exit-code", (action, result)
        finally:
            policy.write_text(original)
            run("systemctl", "reset-failed", "limeos-shadow-containerd")
            run("systemctl", "restart", "limeos-shadow-containerd", check=False)
        shadow_memory = {}
        for unit in [
            "limeos-shadow-core",
            "limeos-shadow-containerd",
            "limeos-shadow-storaged",
        ]:
            pid = (
                main_pid(unit)
                if run("systemctl", "is-active", unit, check=False).stdout.strip()
                == "active"
                else 0
            )
            if pid:
                shadow_memory[unit] = smaps(pid)
        run("apt-get", "remove", "-y", "limeos-shadow", timeout=600)
        assert main_pid("limeos-core") == core_pid, "standard core restarted"
        assert (
            http_request("GET", "/api/v1/auth/session", cookie=state["cookie"])[0]
            == 200
        )
        return {
            "units": units,
            "exec_start": exec_start,
            "refused_results": refused,
            "shadow_memory": shadow_memory,
        }

    @check("final footprint after workload and shadow removal")
    def _():
        time.sleep(30)
        return memory_snapshot()

    RESULT["observations"]["database_bytes"] = {
        p.name: p.stat().st_size for p in Path("/var/lib/limeos/core").glob("*")
    }
    RESULT["observations"]["installed_bytes"] = sum(
        Path(p).stat().st_size
        for p in run("dpkg-query", "-L", "limeos").stdout.splitlines()
        if Path(p).is_file() and not Path(p).is_symlink()
    )
    RESULT["observations"]["journal_limeos_lines"] = len(
        run(
            "journalctl", "-u", "limeos-*", "--no-pager", "-o", "cat"
        ).stdout.splitlines()
    )
    failed = [c["name"] for c in RESULT["checks"] if c["result"] != "pass"]
    RESULT["summary"] = {
        "passed": len(RESULT["checks"]) - len(failed),
        "failed": failed,
    }
    save()
    print("SUMMARY", json.dumps(RESULT["summary"]), flush=True)
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
