#!/usr/bin/env python3
"""Phase 4 footprint gates on the installed, idle candidate; only in the disposable Debian VM.

The guest is put in the state the basic cutover leaves a host in: Docker Engine with a Compose
project of four running members, one created member and two standalone containers; the limeos
package from the signed test repository with one enrolled account; the management catalog from
`limeosctl migrate propose-catalog --install`; and the stack reader started, not enabled, as
activate.sh does. No proxy runs, so measure.sh's two HTTPS rows are not measured here.

Then scripts/cutover/measure.sh (the copy run.py delivers, unchanged) runs as the runbook runs it:
WINDOWS consecutive idle windows of WINDOW_S seconds each, then RESTARTS separate runs with
--measure-core-restart for readiness. Gates (docs/plans/2026-11-19-production-readiness-plan.md,
Phase 4) are applied to measure.sh's own report rows:

  * core plus base executors idle PSS, every window      <= PSS_GATE_BYTES
  * resident disk writes over all windows together        <  DISK_GATE_MB_DAY
  * backend idle CPU over all windows together            <  CPU_CEILING_PERCENT (a regression
    ceiling, not the 0.1 % budget, which this guest does not meet; the result says so)
  * core readiness after a restart, median of the runs    <  READY_GATE_MS

Figures over all windows together (total ticks or bytes over total elapsed time) rather than a
single window: core's metrics store flushes every five minutes, so a short window holds zero or
one flush and its extrapolation swings; 600 s holds about two. The result carries every window's
figure and the per-unit split so a failure can be read without rerunning.
"""

import json
import os
import shutil
import socket
import statistics
import subprocess
import sys
import tarfile
import time
from pathlib import Path

MIB = 1024 * 1024
# Gates. The architecture budgets are 30 MiB, 20 MB/day, 0.1 % of a core and 1 s; the plan gates
# PSS and disk writes tighter so growth is noticed early.
PSS_GATE_BYTES = 15 * MIB
DISK_GATE_MB_DAY = 10.0
READY_GATE_MS = 1000.0
CPU_BUDGET_PERCENT = 0.1
# Not the budget, which this guest does not meet: a regression ceiling. Today's figure here is
# 0.203 % over the 600 s (three local runs, windows 0.18 to 0.225 %; the stack reader takes 0.12
# and core 0.07). Twice that leaves room for a slower or busier runner and still fails when idle
# CPU doubles. See the plan's Phase 4 section.
CPU_CEILING_PERCENT = 0.4

WINDOWS = 3
WINDOW_S = 200
RESTARTS = 3
# Startup work (first passes, catalog load, Docker inventory) is over well before this.
SETTLE_S = 90
# Between readiness runs: core finishes its own startup before it is restarted again.
RESTART_SETTLE_S = 15

MEASURE = Path("/root/limeos-src/scripts/cutover/measure.sh")
OUT = Path("/var/lib/limeos-cutover/footprint")
CTL = "/usr/lib/limeos/limeosctl"
CORE_SOCKET = "/run/limeos-core/core.sock"
CATALOG = Path("/etc/limeos/management-catalog.json")
READER = "limeos-storage-reader.service"
READER_SOCKET = Path("/run/limeos-storage-reader/executor.sock")
BASE_UNITS = ["limeos-storaged.service", "limeos-containerd.service", "limeos-core.service"]
IMAGE = "limeos-footprint:local"
# Nothing listens here: measure.sh's HTTPS rows fail fast instead of timing out.
NO_PROXY_PORT = "8002"


def sh(command, timeout=600, check=True):
    result = subprocess.run(["bash", "-c", command], capture_output=True, text=True, timeout=timeout)
    out = result.stdout + result.stderr
    if check and result.returncode:
        raise RuntimeError(f"command failed ({result.returncode}): {command}\n{out[-3000:]}")
    return result.returncode, out


def eventually(test, timeout, what):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if test():
            return
        time.sleep(0.5)
    raise RuntimeError(f"timed out waiting for {what}")


def active(unit):
    return sh(f"systemctl is-active {unit}", check=False)[1].strip() == "active"


def fixture(repository, package_version):
    sh("apt-get update -qq")
    sh("DEBIAN_FRONTEND=noninteractive apt-get install -y -qq --no-install-recommends docker.io busybox-static curl", timeout=900)
    eventually(lambda: sh("docker info", check=False)[0] == 0, 90, "Docker Engine")
    rootfs = Path("/root/footprint-rootfs.tar")
    with tarfile.open(rootfs, "w") as archive:
        archive.add("/bin/busybox", arcname="bin/busybox")
    sh(f"docker import {rootfs} {IMAGE}")
    stack = Path("/opt/stacks/media")
    stack.mkdir(parents=True)
    services = ["jellyfin", "radarr", "sonarr", "transmission", "airsonic-advanced"]
    (stack / "compose.yaml").write_text("services:\n" + "".join(f"  {s}:\n    image: x\n" for s in services))
    command = "/bin/busybox sleep 2147483647"
    for name in services + ["watchtower", "vpn"]:
        labels = ""
        if name in services:
            labels = (
                f"--label com.docker.compose.project=media --label com.docker.compose.service={name} "
                "--label com.docker.compose.project.working_dir=/opt/stacks/media "
                "--label com.docker.compose.project.config_files=/opt/stacks/media/compose.yaml "
                "--label com.docker.compose.oneoff=False "
            )
        # One member exists but never ran, as a stopped service does.
        verb = "create" if name == "airsonic-advanced" else "run -d"
        sh(f"docker {verb} --name {name} --restart unless-stopped {labels}{IMAGE} {command}")
    sh(f"install -m 0644 {repository}/limeos-archive-keyring.gpg /usr/share/keyrings/limeos-test.gpg")
    Path("/etc/apt/sources.list.d/limeos-test.list").write_text(
        f"deb [signed-by=/usr/share/keyrings/limeos-test.gpg] file:{repository} stable main\n"
    )
    sh("apt-get update -qq")
    sh(f"DEBIAN_FRONTEND=noninteractive apt-get install -y -qq limeos={package_version}", timeout=900)
    eventually(lambda: all(active(u) for u in BASE_UNITS), 60, "the base units")
    eventually(lambda: sh(f"{CTL} --socket {CORE_SOCKET} status", check=False)[0] == 0, 60, "core status")
    token = json.loads(sh(f"{CTL} --socket {CORE_SOCKET} bootstrap")[1])["token"]
    enrolment = json.dumps({"token": token, "username": "footprint-admin", "password": "Synthetic-VM-password-2026"})
    result = subprocess.run([CTL, "--socket", CORE_SOCKET, "enroll"], input=enrolment, text=True, capture_output=True)
    if result.returncode:
        raise RuntimeError("enrolment failed: " + result.stderr[-2000:])
    sh(f"{CTL} migrate propose-catalog --out /root/management-catalog.proposed.json --install")
    if not CATALOG.exists():
        raise RuntimeError("propose-catalog --install did not install the catalog")
    sh(f"systemctl start {READER}")
    eventually(lambda: active(READER) and READER_SOCKET.is_socket(), 60, "the stack reader")


def measure(name, *extra):
    out = OUT / name
    rc, text = sh(
        f"{MEASURE} --host localhost --port {NO_PROXY_PORT} --requests 1 --out {out} " + " ".join(extra),
        timeout=3600,
        check=False,
    )
    report_path = out / "report.json"
    if rc not in (0, 2) or not report_path.exists():
        raise RuntimeError(f"measure.sh {name} produced no report (exit {rc}):\n{text[-3000:]}")
    report = json.loads(report_path.read_text())
    report["exit"] = rc
    return report, text


def rows(report):
    return {m["id"]: m for m in report["measurements"]}


def window_figures(report):
    r = rows(report)
    clk = int(report["meta"]["clk_tck"])
    cpu, disk = r["backend_cpu_idle"], r["resident_disk_writes"]
    elapsed = cpu["elapsed_s"]
    units = {}
    for p in report["processes"]:
        entry = units.setdefault(p["unit"], {"pss_bytes": 0, "cpu_ticks": 0, "write_bytes": 0, "processes": []})
        entry["pss_bytes"] += p["pss_bytes"]
        entry["cpu_ticks"] += p.get("cpu_ticks_delta") or 0
        entry["write_bytes"] += p.get("write_bytes_delta") or 0
        entry["processes"].append(p["comm"])
    for entry in units.values():
        entry["cpu_percent"] = round(entry["cpu_ticks"] / clk / elapsed * 100, 4)
    return {
        "elapsed_s": elapsed,
        "core_base_idle_pss_bytes": r["core_base_idle_pss"]["value"],
        "core_base_idle_swap_bytes": r["core_base_idle_swap"]["value"],
        "all_resident_idle_pss_bytes": r["all_resident_idle_pss"]["value"],
        "backend_cpu_idle_percent": cpu["value"],
        "cpu_ticks": sum(u["cpu_ticks"] for u in units.values()),
        "resident_disk_writes_mb_day": disk["value"],
        "write_bytes": sum(u["write_bytes"] for u in units.values()),
        "row_status": {i: m["status"] for i, m in r.items()},
        "notes": {i: m["note"] for i, m in r.items() if "note" in m},
        "units": units,
        "clk_tck": clk,
    }


def main(package_version):
    if os.geteuid() != 0 or socket.gethostname() != "limeos-p01-test":
        raise SystemExit("Refusing outside disposable VM")
    repository, output = Path(sys.argv[1]), Path(sys.argv[2])
    started = time.monotonic()
    timings = {}
    result = {
        "result": "fail",
        "scope": "Disposable x86-64 Debian 12 KVM guest (2 vCPU, 1 GiB), the installed candidate idle in the "
        "post-activation state; not reference Pi hardware. measure.sh's HTTPS rows are not measured (no proxy).",
        "package_version": package_version,
        "parameters": {"windows": WINDOWS, "window_s": WINDOW_S, "restarts": RESTARTS, "settle_s": SETTLE_S},
    }

    def write():
        result["seconds"] = round(time.monotonic() - started, 1)
        result["timings_s"] = timings
        output.write_text(json.dumps(result, indent=2) + "\n")

    try:
        fixture(repository, package_version)
        timings["fixture"] = round(time.monotonic() - started, 1)
        result["containers"] = sh("docker ps -a --format '{{.Names}} {{.State}}'")[1].split("\n")
        result["units_running"] = [u for u in BASE_UNITS + [READER] if active(u)]
        time.sleep(SETTLE_S)
        if OUT.exists():
            shutil.rmtree(OUT)
        windows, problems = [], []
        mark = time.monotonic()
        for index in range(1, WINDOWS + 1):
            report, _ = measure(f"window-{index}", "--window", str(WINDOW_S))
            figures = window_figures(report)
            windows.append(figures)
            if report["missing_units"] or report["unknown_units"]:
                problems.append(f"window {index}: missing {report['missing_units']}, unknown {report['unknown_units']}")
            for ident in ("core_base_idle_pss", "backend_cpu_idle", "resident_disk_writes"):
                note = figures["notes"].get(ident)
                if note:  # a process set that changed or vanished: not an idle window
                    problems.append(f"window {index} {ident}: {note}")
            if READER not in figures["units"]:
                problems.append(f"window {index}: the stack reader was not measured")
        timings["windows"] = round(time.monotonic() - mark, 1)
        mark = time.monotonic()
        readiness = []
        for index in range(1, RESTARTS + 1):
            time.sleep(RESTART_SETTLE_S)
            report, _ = measure(f"restart-{index}", "--window", "1", "--measure-core-restart")
            row = rows(report)["core_restart_readiness"]
            readiness.append(row["value"])
            if row["value"] is None:
                problems.append(f"restart {index}: {row.get('note')}")
        timings["restarts"] = round(time.monotonic() - mark, 1)
        result["windows"] = windows

        elapsed = sum(w["elapsed_s"] for w in windows)
        clk = windows[0]["clk_tck"]
        cpu = round(sum(w["cpu_ticks"] for w in windows) / clk / elapsed * 100, 4)
        disk = round(sum(w["write_bytes"] for w in windows) / elapsed * 86400 / 1e6, 3)
        pss = [w["core_base_idle_pss_bytes"] for w in windows]
        ready = [r for r in readiness if r is not None]
        units = sorted({u for w in windows for u in w["units"]})
        result["cpu_by_unit_percent"] = {
            u: round(sum(w["units"].get(u, {}).get("cpu_ticks", 0) for w in windows) / clk / elapsed * 100, 4) for u in units
        }
        result["write_bytes_by_unit"] = {u: sum(w["units"].get(u, {}).get("write_bytes", 0) for w in windows) for u in units}
        result["readiness_ms"] = readiness
        gates = {
            "core_base_idle_pss": {
                "value_bytes": max(pss) if None not in pss else None,
                "value_mib": round(max(pss) / MIB, 2) if None not in pss else None,
                "per_window_mib": [None if p is None else round(p / MIB, 2) for p in pss],
                "gate": "every window <= 15 MiB",
                "pass": None not in pss and max(pss) <= PSS_GATE_BYTES,
            },
            "resident_disk_writes": {
                "value_mb_day": disk,
                "per_window_mb_day": [w["resident_disk_writes_mb_day"] for w in windows],
                "gate": f"all windows together < {DISK_GATE_MB_DAY} MB/day",
                "pass": disk < DISK_GATE_MB_DAY,
            },
            "backend_cpu_idle": {
                "value_percent": cpu,
                "per_window_percent": [w["backend_cpu_idle_percent"] for w in windows],
                "gate": f"all windows together < {CPU_CEILING_PERCENT} % of a core (regression ceiling)",
                "pass": cpu < CPU_CEILING_PERCENT,
                "budget_percent": CPU_BUDGET_PERCENT,
                "budget_met": cpu < CPU_BUDGET_PERCENT,
                "budget_note": "The 0.1 % budget is not gated. The gate is a regression ceiling over today's figure; "
                "budget_met says whether this run met the budget.",
            },
            "core_restart_readiness": {
                "value_ms": statistics.median(ready) if len(ready) == RESTARTS else None,
                "per_run_ms": readiness,
                "gate": f"median of {RESTARTS} restarts < {READY_GATE_MS:.0f} ms (includes the stop)",
                "pass": len(ready) == RESTARTS and statistics.median(ready) < READY_GATE_MS,
            },
        }
        result["gates"] = gates
        result["problems"] = problems
        result["ungated"] = {
            "core_base_idle_swap": "recorded per window; the guest has no swap",
            "health_http_p95 and cached_observation_http_p95": "no proxy in this guest; cached overview p95 is gated on the Pi figure only",
            "installed_footprint": "measured by measure.sh (see the window reports), not a Phase 4 gate",
            "storage inventory p95, mount admission": "failing or deferred in the plan; not measured by measure.sh",
            "an unchanged poll writes zero bytes": "measure.sh measures windows, not polls; gated by the check job's "
            "stable_real_runtime_passes_leave_process_write_bytes_unchanged test",
        }
        result["reports"] = {
            p.name: json.loads((p / "report.json").read_text())["measurements"] for p in sorted(OUT.iterdir())
        }
        failed = [name for name, gate in gates.items() if not gate["pass"]]
        result["result"] = "pass" if not failed and not problems else "fail"
        result["failed_gates"] = failed
        for name, gate in gates.items():
            print(("PASS " if gate["pass"] else "FAIL ") + f"{name}: {json.dumps({k: v for k, v in gate.items() if k != 'budget_note'})}", flush=True)
        for problem in problems:
            print("FAIL " + problem, flush=True)
        if not gates["backend_cpu_idle"]["budget_met"]:
            print(f"NOTE idle CPU {cpu} % does not meet the {CPU_BUDGET_PERCENT} % budget (not gated)", flush=True)
    except BaseException as error:  # noqa: BLE001 - recorded in the result, then re-raised
        result["error"] = repr(error)[:4000]
        write()
        raise
    write()
    if result["result"] != "pass":
        raise SystemExit("footprint gates failed: " + ", ".join(result["failed_gates"] + result["problems"]))


if __name__ == "__main__":
    main(package_version=sys.argv[3] if len(sys.argv) > 3 else "0.4.4")
