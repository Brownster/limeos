#!/usr/bin/env python3
"""Phase 4 footprint gates on the installed, idle candidate; only in the disposable Debian VM.

The guest is put in the state the basic cutover leaves a host in: Docker Engine with a Compose
project of four running members, one created member and two standalone containers; the limeos
package from the signed test repository with one enrolled account; the management catalog from
`limeosctl migrate propose-catalog --install`; and the stack reader started, not enabled, as
activate.sh does. No proxy runs, so measure.sh's two HTTPS rows are not measured here.

Then scripts/cutover/measure.sh (the copy run.py delivers, unchanged) runs as the runbook runs it:
WINDOWS consecutive idle windows of WINDOW_S seconds each, then RESTARTS separate runs with
--measure-core-restart for readiness. The gated figures are measure.sh's own row values: PSS per
window, and CPU and disk writes as each window's row value weighted by its elapsed time:

  * core plus base executors idle PSS, every window      <= PSS_GATE_BYTES
  * resident disk writes over all windows together        <  DISK_GATE_MB_DAY
  * backend idle CPU over all windows together            <  CPU_CEILING_PERCENT (a regression
    ceiling, not the 0.1 % budget, which this guest does not meet; the result says so)
  * core readiness after `systemctl restart`, median      <  READY_GATE_MS

A row value alone can pass vacuously (measure.sh sums only the processes it found), so every
window must also show each base unit (measure.sh's own list, from lib.sh) and the stack reader
with its systemd main process measured at both ends: PSS above zero and readable CPU and write
deltas; the per-process figures must add up to the rows; no main process or restart count may
change. A changed process set fails the window unless every changed entry is a helper of the
stack reader (not its main process), which the reader starts for its 30 s stack refresh. Such a
helper's CPU is not lost: the reader waits for it, so its whole CPU time joins the reader's
cutime and cstime, which measure.sh counts, in the window where it ends; only a helper still
running at the last sample is left out, and its ticks at that sample are recorded. Over the
whole run core and the reader must have used CPU and core must have written (its metrics store
flushes every five minutes). See window_figures and evaluate.

Figures over all windows together rather than a single window: core's metrics store flushes every
five minutes, so a 200 s window holds zero or one flush and its extrapolation swings; 600 s
normally holds two. The result carries every window's figure and the per-unit split, pass or fail.
"""

import http.client
import json
import os
import pwd
import re
import shutil
import socket
import sqlite3
import statistics
import subprocess
import sys
import tarfile
import time
from pathlib import Path

MIB = 1024 * 1024
# Gates. The architecture budgets are 30 MiB, 20 MB/day, 0.1 % of a core and 1 s.
# PSS: core plus base executors settled on a month-old, used store read 13.94 to 14.31 MiB over
# nine runs (six on this laptop's guest, ascending and descending seeds; one on the GitHub runner,
# 14.26 MiB; two traces), core 10.67 to 11.06 MiB. The spread, 0.37 MiB, is mostly the
# executors' shared code pages. 15.5 MiB is 1.19 MiB above the highest, three times that spread,
# so noise does not fail it and a regression of 1.5 MiB does; it is half the budget.
PSS_GATE_BYTES = int(15.5 * MIB)
# Disk writes: a steady metrics flush on a month-old store writes 24 KiB of write_bytes (two WAL
# frames, one for the leaf of the inserted rows and one for the leaf of the deleted oldest rows,
# and the WAL index) every five minutes: 7.07 MB/day, which every run read. The 600 s normally
# holds two flushes, sometimes three (10.6 MB/day), and a checkpoint every 64 flushes adds about
# 12 KiB (12.4 MB/day if both fall in one run). Below 20 MB/day, 13 MB/day passes those, and fails
# when a steady flush passes 45 KiB (1.9 times today's) with two flushes in the run, or 30 KiB with
# three, or when another idle writer adds about 6 MB/day.
DISK_GATE_MB_DAY = 13.0
READY_GATE_MS = 1000.0
CPU_BUDGET_PERCENT = 0.1
# Not the budget, which neither guest meets: a regression ceiling over the measured figures.
# Over the 600 s, the GitHub runner's guest read 0.148 % and 0.188 % in two runs (core 0.075 and
# 0.103, reader 0.058 and 0.070) and this laptop's 0.200 to 0.210 % in nine (reader about 0.12,
# core 0.07). 0.3 % is 1.43 times the highest: a doubling of idle CPU fails it anywhere.
CPU_CEILING_PERCENT = 0.3
# The per-process sums must match measure.sh's rows to their rounding (4 and 3 decimals).
CPU_AGREEMENT = 0.0005
DISK_AGREEMENT = 0.002

WINDOWS = 3
WINDOW_S = 200
RESTARTS = 3
# From the end of the use pass, which follows core's restart on the seeded store. Core's first
# metrics flush after a start comes 4 to 5 minutes in (five one-minute samples) and writes twice
# what a steady flush does (the WAL and its index are created again: 48 KiB against 24 KiB in the
# VM trace), so the windows start after it; startup's other work is over long before.
SETTLE_S = 360
# Between readiness runs: core finishes its own startup before it is restarted again.
RESTART_SETTLE_S = 15
# Every call is bounded, and the whole suite too, so that a hang ends here with the result
# written rather than when the CI job is cancelled. A normal run takes about 1100 s.
SUITE_BUDGET_S = 1500
WINDOW_BOUND_S = WINDOW_S + 180
RESTART_BOUND_S = 120

MEASURE = Path("/root/limeos-src/scripts/cutover/measure.sh")
OUT = Path("/var/lib/limeos-cutover/footprint")
CTL = "/usr/lib/limeos/limeosctl"
CORE_SOCKET = "/run/limeos-core/core.sock"
CATALOG = Path("/etc/limeos/management-catalog.json")
CORE = "limeos-core.service"
READER = "limeos-storage-reader.service"
READER_SOCKET = Path("/run/limeos-storage-reader/executor.sock")
IMAGE = "limeos-footprint:local"
# Nothing listens here: measure.sh's HTTPS rows fail fast instead of timing out.
NO_PROXY_PORT = "8002"
CHANGE_NOTE = "process set changed during the window"
THP_ENABLED = Path("/sys/kernel/mm/transparent_hugepage/enabled")
METRICS = Path("/var/lib/limeos/core/metrics.sqlite")
# A host that has run past a month: core's metrics store holds the 31 days it keeps, one sample a
# minute, written as core writes them (telemetry.rs): every five minutes one transaction inserts
# the five newest samples and deletes those older than 31 days. Seeding 35 days leaves four days of
# that pruning (and of SQLite reusing the freed pages) behind the 31 kept.
RETENTION_S = 31 * 86400
SEEDED_DAYS = 35
FLUSH_SAMPLES = 5
LOGIN = {"username": "footprint-admin", "password": "Synthetic-VM-password-2026"}
GATED_ROWS = ("core_base_idle_pss", "backend_cpu_idle", "resident_disk_writes")

deadline = None


# ---------------------------------------------------------------- pure: report to figures to gates
def base_units_of(report):
    """measure.sh's base units (lib.sh RUST_UNITS), as it recorded them."""
    return report.get("meta", {}).get("base_units", "").split()


def lib_base_units(lib_text):
    """RUST_UNITS from scripts/cutover/lib.sh, the list measure.sh calls its base units."""
    match = re.search(r"^RUST_UNITS=\(([^)]*)\)", lib_text, re.MULTILINE)
    if not match or not match.group(1).split():
        raise RuntimeError("RUST_UNITS not found in lib.sh")
    return match.group(1).split()


def parse_samples(raw):
    """{"start"|"end": {(unit, pid, starttime): (ticks, write_bytes|None)}} from measure.sh's raw.txt."""
    samples = {"start": {}, "end": {}}
    for line in raw.splitlines():
        f = line.split()
        if len(f) == 7 and f[0] == "sample" and f[1] in samples:
            samples[f[1]][(f[2], f[3], f[4])] = (int(f[5]), None if f[6] == "NA" else int(f[6]))
    return samples


def window_figures(report, raw, before, after, memory=None):
    """One idle window's figures and problems.

    `before` and `after`: {unit: {"MainPID": str, "NRestarts": str}} from systemctl show around the
    window, for every base unit and the reader. `memory`: {unit: smaps_rollup fields in kB} of each
    of those main processes, read just after the window, so a step can be attributed to anonymous
    or file-backed memory; a unit with transparent huge pages in use fails the window (the guest
    disables them: the production kernel has none). A problem is anything that makes the window's
    figures untrustworthy; the caller fails the suite on any.
    """
    problems, tolerated = [], []
    base = base_units_of(report)
    required = [*base, READER]
    if CORE not in base:
        problems.append(f"measure.sh's base units do not include {CORE}: {base}")
    if report.get("missing_units") or report.get("unknown_units"):
        problems.append(f"missing units {report.get('missing_units')}, unknown units {report.get('unknown_units')}")
    rows = {m["id"]: m for m in report.get("measurements", [])}
    for ident in GATED_ROWS:
        row = rows.get(ident)
        if row is None or row.get("value") is None:
            problems.append(f"{ident} has no value ({row.get('note') if row else 'row absent'})")
            continue
        note = row.get("note")
        if note and not note.startswith(CHANGE_NOTE):
            problems.append(f"{ident}: {note}")
    pss_row = rows.get("core_base_idle_pss", {}).get("value")
    if pss_row is not None and pss_row <= 0:
        problems.append(f"core_base_idle_pss is {pss_row}")
    cpu_row = rows.get("backend_cpu_idle", {})
    elapsed = cpu_row.get("elapsed_s") or 0
    if elapsed <= 0:
        problems.append(f"elapsed time is {elapsed}")
    clk = int(report.get("meta", {}).get("clk_tck", "100"))

    # systemd: the same main process, no restart, for every unit that matters.
    for unit in required:
        b, a = before.get(unit), after.get(unit)
        if not b or not a:
            problems.append(f"{unit}: no systemctl state around the window")
            continue
        if b.get("MainPID") in (None, "", "0") or b != a:
            problems.append(f"{unit}: main process or restart count changed or absent: before {b}, after {a}")

    # The process set: only a helper of the reader may come or go.
    samples = parse_samples(raw)
    changed = sorted(set(samples["start"]) ^ set(samples["end"]))
    reader_main = (before.get(READER) or {}).get("MainPID")
    for key in changed:
        unit, pid, start = key
        phase = "start" if key in samples["start"] else "end"
        if unit == READER and pid != reader_main:
            ticks = samples[phase][key][0]
            tolerated.append({"unit": unit, "pid": int(pid), "only_at": phase, "cpu_ticks_then": ticks})
        else:
            problems.append(f"process set changed: {unit} pid {pid} only at the {phase} sample")

    # Each required unit's main process measured at both ends, with memory, CPU and writes.
    processes = report.get("processes", [])
    units = {}
    for unit in required:
        main = (before.get(unit) or {}).get("MainPID")
        entry = next((p for p in processes if p.get("unit") == unit and str(p.get("pid")) == main), None)
        if entry is None:
            problems.append(f"{unit}: main process {main} not in measure.sh's processes")
        elif not entry.get("pss_bytes") or entry.get("cpu_ticks_delta") is None or entry.get("write_bytes_delta") is None:
            problems.append(f"{unit}: main process {main} measured without PSS, CPU or write delta: {entry}")
    for p in processes:
        u = units.setdefault(p["unit"], {"pss_bytes": 0, "cpu_ticks": 0, "write_bytes": 0, "processes": []})
        u["pss_bytes"] += p.get("pss_bytes") or 0
        u["cpu_ticks"] += p.get("cpu_ticks_delta") or 0
        u["write_bytes"] += p.get("write_bytes_delta") or 0
        u["processes"].append([p.get("pid"), p.get("comm")])
    if elapsed > 0:
        for u in units.values():
            u["cpu_percent"] = round(u["cpu_ticks"] / clk / elapsed * 100, 4)

    # The per-process sums must be the rows: a process measure.sh counted but did not list (or the
    # reverse) would make the split, and the floors below, wrong.
    ticks = sum(u["cpu_ticks"] for u in units.values())
    written = sum(u["write_bytes"] for u in units.values())
    disk_row = rows.get("resident_disk_writes", {}).get("value")
    if elapsed > 0 and cpu_row.get("value") is not None:
        if abs(ticks / clk / elapsed * 100 - cpu_row["value"]) > CPU_AGREEMENT:
            problems.append(f"per-process CPU {ticks} ticks disagrees with the row {cpu_row['value']} %")
    if elapsed > 0 and disk_row is not None:
        if abs(written / elapsed * 86400 / 1e6 - disk_row) > DISK_AGREEMENT:
            problems.append(f"per-process writes {written} bytes disagree with the row {disk_row} MB/day")
    base_pss = sum(u["pss_bytes"] for name, u in units.items() if name in base)
    if pss_row is not None and base_pss != pss_row:
        problems.append(f"per-process base PSS {base_pss} disagrees with the row {pss_row}")

    memory = memory or {}
    for unit in required:
        detail = memory.get(unit)
        if not detail:
            problems.append(f"{unit}: no smaps_rollup after the window")
        elif detail.get("AnonHugePages", 0) != 0:
            problems.append(f"{unit}: {detail['AnonHugePages']} kB of transparent huge pages; the guest setting did not take")

    return {
        "elapsed_s": elapsed,
        "clk_tck": clk,
        "memory_kb": memory,
        "core_base_idle_pss_bytes": pss_row,
        "core_base_idle_swap_bytes": rows.get("core_base_idle_swap", {}).get("value"),
        "all_resident_idle_pss_bytes": rows.get("all_resident_idle_pss", {}).get("value"),
        "backend_cpu_idle_percent": cpu_row.get("value"),
        "resident_disk_writes_mb_day": disk_row,
        "cpu_ticks": ticks,
        "write_bytes": written,
        "units": units,
        "tolerated_reader_helpers": tolerated,
        "systemd": {"before": before, "after": after},
        "row_status": {i: m.get("status") for i, m in rows.items()},
        "notes": {i: m["note"] for i, m in rows.items() if "note" in m},
        "problems": problems,
    }


def readiness_of(report):
    """(milliseconds or None, note) from one --measure-core-restart report."""
    row = next((m for m in report.get("measurements", []) if m["id"] == "core_restart_readiness"), None)
    if row is None:
        return None, "core_restart_readiness row absent"
    return row.get("value"), row.get("note")


def evaluate(windows, readiness, thp=None):
    """Gates and whole-run problems from window_figures results and readiness values."""
    problems = [f"window {i}: {p}" for i, w in enumerate(windows, 1) for p in w["problems"]]
    if thp is not None and "[never]" not in thp:
        problems.append(f"transparent huge pages are not disabled in the guest: {thp}")
    if len(windows) != WINDOWS:
        problems.append(f"{len(windows)} windows measured, {WINDOWS} required")
    timed = [w for w in windows if w["elapsed_s"] > 0]
    elapsed = sum(w["elapsed_s"] for w in timed)
    cpu = disk = None
    if timed and all(w["backend_cpu_idle_percent"] is not None for w in timed):
        cpu = round(sum(w["backend_cpu_idle_percent"] * w["elapsed_s"] for w in timed) / elapsed, 4)
    if timed and all(w["resident_disk_writes_mb_day"] is not None for w in timed):
        disk = round(sum(w["resident_disk_writes_mb_day"] * w["elapsed_s"] for w in timed) / elapsed, 3)
    pss = [w["core_base_idle_pss_bytes"] for w in windows]
    clk = windows[0]["clk_tck"] if windows else 100
    names = sorted({u for w in windows for u in w["units"]})
    cpu_by_unit = {u: round(sum(w["units"].get(u, {}).get("cpu_ticks", 0) for w in timed) / clk / elapsed * 100, 4)
                   for u in names} if elapsed else {}
    ticks_by_unit = {u: sum(w["units"].get(u, {}).get("cpu_ticks", 0) for w in windows) for u in names}
    writes_by_unit = {u: sum(w["units"].get(u, {}).get("write_bytes", 0) for w in windows) for u in names}
    # Floors today's behaviour guarantees over the 600 s: core's passes and the reader's 30 s
    # refresh use CPU, and core's metrics store flushes at least once.
    if windows:
        if ticks_by_unit.get(CORE, 0) <= 0:
            problems.append(f"{CORE} used no CPU over the run")
        if writes_by_unit.get(CORE, 0) <= 0:
            problems.append(f"{CORE} wrote nothing over the run")
        if ticks_by_unit.get(READER, 0) <= 0:
            problems.append(f"{READER} used no CPU over the run")
    ready = [r for r in readiness if r is not None]
    pss_ok = bool(pss) and None not in pss and max(pss) <= PSS_GATE_BYTES
    gates = {
        "core_base_idle_pss": {
            "value_bytes": max(pss) if pss and None not in pss else None,
            "value_mib": round(max(pss) / MIB, 2) if pss and None not in pss else None,
            "per_window_mib": [None if p is None else round(p / MIB, 2) for p in pss],
            "gate": f"every window <= {PSS_GATE_BYTES / MIB:g} MiB",
            "pass": pss_ok,
        },
        "resident_disk_writes": {
            "value_mb_day": disk,
            "per_window_mb_day": [w["resident_disk_writes_mb_day"] for w in windows],
            "gate": f"measure.sh's rows weighted by elapsed time < {DISK_GATE_MB_DAY} MB/day",
            "pass": disk is not None and disk < DISK_GATE_MB_DAY,
        },
        "backend_cpu_idle": {
            "value_percent": cpu,
            "per_window_percent": [w["backend_cpu_idle_percent"] for w in windows],
            "by_unit_percent": cpu_by_unit,
            "gate": f"measure.sh's rows weighted by elapsed time < {CPU_CEILING_PERCENT} % of a core (regression ceiling)",
            "pass": cpu is not None and cpu < CPU_CEILING_PERCENT,
            "budget_percent": CPU_BUDGET_PERCENT,
            "budget_met": cpu is not None and cpu < CPU_BUDGET_PERCENT,
            "budget_note": "The 0.1 % budget is not gated. The gate is a regression ceiling over today's figure; "
            "budget_met says whether this run met the budget.",
        },
        "core_restart_readiness": {
            "value_ms": statistics.median(ready) if len(ready) == RESTARTS else None,
            "per_run_ms": list(readiness),
            "gate": f"median of {RESTARTS} runs < {READY_GATE_MS:.0f} ms (systemctl restart then limeosctl status; "
            "warm cache, the stop included)",
            "pass": len(readiness) == RESTARTS and len(ready) == RESTARTS and statistics.median(ready) < READY_GATE_MS,
        },
    }
    failed = [name for name, gate in gates.items() if not gate["pass"]]
    return {
        "result": "pass" if not failed and not problems else "fail",
        "failed_gates": failed,
        "problems": problems,
        "gates": gates,
        "cpu_by_unit_percent": cpu_by_unit,
        "cpu_ticks_by_unit": ticks_by_unit,
        "write_bytes_by_unit": writes_by_unit,
    }


# ---------------------------------------------------------------- guest side
def remaining(bound):
    left = deadline - time.monotonic() if deadline is not None else bound
    if left <= 1:
        raise RuntimeError(f"suite budget of {SUITE_BUDGET_S} s spent")
    return min(bound, left)


def sh(command, timeout, check=True):
    try:
        result = subprocess.run(["bash", "-c", command], capture_output=True, text=True, timeout=remaining(timeout))
    except subprocess.TimeoutExpired as error:
        raise RuntimeError(f"timed out after {error.timeout:.0f} s: {command}") from None
    out = result.stdout + result.stderr
    if check and result.returncode:
        raise RuntimeError(f"command failed ({result.returncode}): {command}\n{out[-3000:]}")
    return result.returncode, out


def eventually(test, timeout, what):
    end = time.monotonic() + remaining(timeout)
    while time.monotonic() < end:
        if test():
            return
        time.sleep(0.5)
    raise RuntimeError(f"timed out waiting for {what}")


def active(unit):
    return sh(f"systemctl is-active {unit}", 30, check=False)[1].strip() == "active"


def unit_states(units):
    states = {}
    for unit in units:
        out = sh(f"systemctl show {unit} -p MainPID -p NRestarts", 30)[1]
        states[unit] = dict(line.split("=", 1) for line in out.splitlines() if "=" in line)
    return states


def disable_thp():
    """x86's 2 MiB transparent huge pages back a whole 2 MiB of a process's heap once one page of an
    aligned block is touched, which made core's PSS step by 2 MiB at random in one CI run. The
    production Pi's kernel has no transparent huge pages (and 16 KiB pages), so they are off here
    before LimeOS starts; a window that still finds them fails."""
    for name in ("enabled", "defrag"):
        (THP_ENABLED.parent / name).write_text("never")
    return THP_ENABLED.read_text().strip()


def seeded_flushes(now):
    """The flushes core would have written over the `SEEDED_DAYS` before `now`, oldest first:
    (five (minute, cpu, memory, temperature, disk) rows, ascending, and the cutoff its `DELETE`
    used, the newest row's time less 31 days)."""
    last = now // 60 * 60 - 60
    minutes = range(last - (SEEDED_DAYS * 1440 - 1) * 60, last + 60, 60)
    rows = [(m, 12.5, 40.0, 51.0, 63.0) for m in minutes]
    return [(rows[i:i + FLUSH_SAMPLES], rows[i + FLUSH_SAMPLES - 1][0] - RETENTION_S)
            for i in range(0, len(rows) - FLUSH_SAMPLES + 1, FLUSH_SAMPLES)]


def seed_metrics():
    """Stop core, write its metrics store's last five weeks as core writes them, start it again:
    the measured core opens a store laid out as a month-old host's is. Core's unit allows ten
    starts in six hours; the suite starts it twice between resets (postinst's start on install,
    this one), and measure.sh resets the count before each of its three restarts."""
    sh("systemctl stop limeos-core", 60)
    db = sqlite3.connect(METRICS, isolation_level=None)
    # The connection settings core's own uses (telemetry.rs), so pages split, free and are
    # checkpointed back as they would be.
    db.execute("PRAGMA synchronous=NORMAL")
    db.execute("PRAGMA wal_autocheckpoint=128")
    for rows, cutoff in seeded_flushes(int(time.time())):
        db.execute("BEGIN")
        db.executemany("INSERT OR REPLACE INTO samples VALUES(?,?,?,?,?)", rows)
        db.execute("DELETE FROM samples WHERE at<?", (cutoff,))
        db.execute("COMMIT")
    db.close()
    owner = METRICS.parent.stat()
    for path in METRICS.parent.glob(METRICS.name + "*"):
        os.chown(path, owner.st_uid, owner.st_gid)
    sh("systemctl start limeos-core", 60)
    eventually(lambda: sh(f"{CTL} --socket {CORE_SOCKET} status", 30, check=False)[0] == 0, 60, "core status")
    return METRICS.stat().st_size


def request(method, path, body=None, cookie=None):
    connection = http.client.HTTPConnection("127.0.0.1", 8003, timeout=10)
    headers = {"Origin": "https://localhost", "Content-Type": "application/json"}
    if cookie:
        headers["Cookie"] = cookie
    try:
        connection.request(method, path, json.dumps(body) if body else None, headers)
        response = connection.getresponse()
        response.read()
        return response.status, dict(response.getheaders())
    finally:
        connection.close()


def use_pass():
    """What a household does first: log in, open the overview, look at the month's history.
    Fills what that use fills (allocator arenas, caches) before the settle, so the windows
    measure core as it is after use, not a process nobody has asked anything."""
    status, headers = request("POST", "/api/v1/auth/login", LOGIN)
    if status != 200:
        raise RuntimeError(f"login for the use pass failed: {status}")
    cookie = headers["set-cookie"].split(";")[0]
    codes = {}
    for path in ["/api/v1/overview", "/api/v1/resources?limit=50", "/api/v1/system/history?range=30d",
                 "/api/v1/system/history?range=24h", "/api/v1/overview", "/api/v1/system/history?range=30d"]:
        codes[path] = request("GET", path, cookie=cookie)[0]
    if any(code != 200 for code in codes.values()):
        raise RuntimeError(f"a use-pass request failed: {codes}")
    return codes


def rollup(pid):
    out = {}
    for line in Path(f"/proc/{pid}/smaps_rollup").read_text().splitlines()[1:]:
        key, value = line.split(":", 1)
        out[key] = int(value.split()[0])
    return {k: out.get(k) for k in ("Rss", "Pss", "Pss_Anon", "Pss_File", "AnonHugePages")}


def fixture(repository, package_version):
    sh("apt-get update -qq", 180)
    sh("DEBIAN_FRONTEND=noninteractive apt-get install -y -qq --no-install-recommends docker.io busybox-static curl", 300)
    eventually(lambda: sh("docker info", 30, check=False)[0] == 0, 90, "Docker Engine")
    rootfs = Path("/root/footprint-rootfs.tar")
    with tarfile.open(rootfs, "w") as archive:
        archive.add("/bin/busybox", arcname="bin/busybox")
    sh(f"docker import {rootfs} {IMAGE}", 60)
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
        sh(f"docker {verb} --name {name} --restart unless-stopped {labels}{IMAGE} {command}", 60)
    sh(f"install -m 0644 {repository}/limeos-archive-keyring.gpg /usr/share/keyrings/limeos-test.gpg", 30)
    Path("/etc/apt/sources.list.d/limeos-test.list").write_text(
        f"deb [signed-by=/usr/share/keyrings/limeos-test.gpg] file:{repository} stable main\n"
    )
    sh("apt-get update -qq", 120)
    sh(f"DEBIAN_FRONTEND=noninteractive apt-get install -y -qq limeos={package_version}", 180)
    eventually(lambda: all(active(u) for u in ("limeos-storaged", "limeos-containerd", "limeos-core")), 60, "the base units")
    eventually(lambda: sh(f"{CTL} --socket {CORE_SOCKET} status", 30, check=False)[0] == 0, 60, "core status")
    token = json.loads(sh(f"{CTL} --socket {CORE_SOCKET} bootstrap", 30)[1])["token"]
    enrolment = json.dumps({"token": token, "username": "footprint-admin", "password": "Synthetic-VM-password-2026"})
    result = subprocess.run([CTL, "--socket", CORE_SOCKET, "enroll"], input=enrolment, text=True,
                            capture_output=True, timeout=remaining(30))
    if result.returncode:
        raise RuntimeError("enrolment failed: " + result.stderr[-2000:])
    sh(f"{CTL} migrate propose-catalog --out /root/management-catalog.proposed.json --install", 60)
    if not CATALOG.exists():
        raise RuntimeError("propose-catalog --install did not install the catalog")
    sh(f"systemctl start {READER}", 60)
    eventually(lambda: active(READER) and READER_SOCKET.is_socket(), 60, "the stack reader")
    reader = reader_serves_core()
    return {"metrics_bytes": seed_metrics(), "use_pass": use_pass(), "reader": reader}


def reader_serves_core():
    """PR-32, gating: the reader runs as its own account with exactly its capability sets and
    groups, is dumpable (its /proc entry is the account's), and core takes its answers. Core's
    stack status turns fresh from both the catalog and the Engine (the stack worker reads the
    Engine over the docker group with no capability); core's storage inventory names this guest's
    disk with its filesystem identified by blkid through /proc/PID/fd (the reason the capability is
    ambient); and a container-health read as core's account answers for this guest's containers
    (the health worker, again with no capability)."""
    import secrets

    from guest import executor_probe_code
    from p04_fixture_readiness import READER_ACCOUNT, reader_confined, reader_status

    account = pwd.getpwnam(READER_ACCOUNT).pw_uid
    pid = int(sh(f"systemctl show {READER} -p MainPID --value", 30)[1])
    if not reader_confined(reader_status(pid)):
        raise RuntimeError(f"{READER} is not {READER_ACCOUNT} with exactly CAP_DAC_READ_SEARCH: {reader_status(pid)}")
    if Path(f"/proc/{pid}").stat().st_uid != account:
        raise RuntimeError(f"{READER} is not dumpable: its /proc entry is not owned by {READER_ACCOUNT}")
    if READER_SOCKET.lstat().st_uid != account:
        raise RuntimeError(f"{READER_SOCKET} is not owned by {READER_ACCOUNT}")
    status, headers = request("POST", "/api/v1/auth/login", LOGIN)
    if status != 200:
        raise RuntimeError(f"login for the reader check failed: {status}")
    cookie = headers["set-cookie"].split(";")[0]

    def get(path):
        connection = http.client.HTTPConnection("127.0.0.1", 8003, timeout=10)
        try:
            connection.request("GET", path, headers={"Cookie": cookie, "Origin": "https://localhost"})
            response = connection.getresponse()
            body = response.read()
        finally:
            connection.close()
        return response.status, (json.loads(body) if body else None)

    seen = {}

    def fresh():
        code, value = get("/api/v1/deployment/stacks")
        if code != 200:
            return False
        seen.update(catalog=value["catalog"]["state"], engine=value["engine"]["state"])
        return seen == {"catalog": "fresh", "engine": "fresh"} and any(s["project"] == "media" for s in value["stacks"])

    eventually(fresh, 75, f"core's stack status from {READER}")
    devices = []

    def inventory():
        code, value = get("/api/v1/storage/inventory")
        if code != 200:
            return False
        devices[:] = value["inventory"]["devices"]
        return True

    eventually(inventory, 60, f"core's storage inventory from {READER}")
    identified = [d for d in devices if d["filesystem"] and d["filesystem_uuid"]]
    if not any(d["boot_backing"] and d["filesystem"] == "ext4" for d in identified):
        raise RuntimeError(f"the storage inventory names no ext4 root filesystem identified by blkid: {devices}")
    nonce = secrets.token_hex(32)
    reply = subprocess.run(
        ["runuser", "-u", "limeos-core", "-g", "limeos-host-access", "--", "python3", "-c",
         executor_probe_code(), str(READER_SOCKET),
         json.dumps({"operation": "container_health", "version": 1, "nonce": nonce})],
        capture_output=True, text=True, timeout=remaining(30), check=False,
    )
    health = json.loads(reply.stdout) if reply.stdout.strip() else {}
    cohort = health.get("container_health") or {}
    if health.get("ready") is not True or health.get("error") is not None or cohort.get("nonce") != nonce \
            or len(cohort.get("members", [])) < 6:
        raise RuntimeError(f"container-health read from {READER} failed: {reply.stdout[-1500:]}{reply.stderr[-500:]}")
    return {
        "account": READER_ACCOUNT,
        "stacks": seen,
        "filesystems": sorted({(d["filesystem"], d["boot_backing"]) for d in identified}),
        "health_members": len(cohort["members"]),
    }

def measure(name, bound, *extra):
    out = OUT / name
    rc, text = sh(
        f"{MEASURE} --host localhost --port {NO_PROXY_PORT} --requests 1 --out {out} " + " ".join(extra),
        bound,
        check=False,
    )
    report_path = out / "report.json"
    if rc not in (0, 2) or not report_path.exists():
        raise RuntimeError(f"measure.sh {name} produced no report (exit {rc}):\n{text[-3000:]}")
    report = json.loads(report_path.read_text())
    report["exit"] = rc
    return report, (out / "raw.txt").read_text()


def main(package_version):
    global deadline
    if os.geteuid() != 0 or socket.gethostname() != "limeos-p01-test":
        raise SystemExit("Refusing outside disposable VM")
    repository, output = Path(sys.argv[1]), Path(sys.argv[2])
    started = time.monotonic()
    deadline = started + SUITE_BUDGET_S
    timings = {}
    windows, readiness = [], []
    result = {
        "result": "fail",
        "scope": "Disposable x86-64 Debian 12 KVM guest (2 vCPU, 1 GiB), the installed candidate idle in the "
        "post-activation state; not reference Pi hardware. measure.sh's HTTPS rows are not measured (no proxy).",
        "package_version": package_version,
        "parameters": {"windows": WINDOWS, "window_s": WINDOW_S, "restarts": RESTARTS, "settle_s": SETTLE_S,
                       "suite_budget_s": SUITE_BUDGET_S},
        "windows": windows,
        "readiness_ms": readiness,
        "ungated": {
            "core_base_idle_swap": "recorded per window; the guest has no swap",
            "health_http_p95 and cached_observation_http_p95": "no proxy in this guest; cached overview p95 is gated on the Pi figure only",
            "installed_footprint, all_resident_idle_pss": "recorded in the window reports, not Phase 4 gates",
            "storage inventory p95, mount admission": "failing or deferred in the plan; not measured by measure.sh",
            "an unchanged poll writes zero bytes": "measure.sh measures windows, not polls; gated by the check job's "
            "stable_real_runtime_passes_leave_process_write_bytes_unchanged test",
        },
    }

    def write():
        # Whatever was measured so far is evaluated too, so a failed or cut-short run still says
        # what each window and unit used.
        result.update(evaluate(windows, readiness, result.get("thp")))
        if "error" in result:
            result["result"] = "fail"
        result["seconds"] = round(time.monotonic() - started, 1)
        result["timings_s"] = timings
        if OUT.exists():
            result["reports"] = {p.name: json.loads((p / "report.json").read_text())["measurements"]
                                 for p in sorted(OUT.iterdir()) if (p / "report.json").exists()}
        output.write_text(json.dumps(result, indent=2) + "\n")

    try:
        result["thp"] = disable_thp()
        result["state"] = fixture(repository, package_version)
        timings["fixture"] = round(time.monotonic() - started, 1)
        print(f"fixture ready after {timings['fixture']} s; settling {SETTLE_S} s", flush=True)
        result["containers"] = sh("docker ps -a --format '{{.Names}} {{.State}}'", 30)[1].split("\n")
        time.sleep(SETTLE_S)
        if OUT.exists():
            shutil.rmtree(OUT)
        mark = time.monotonic()
        # The same base units measure.sh measures: lib.sh's RUST_UNITS, read from the copy it sources.
        required = [*lib_base_units((MEASURE.parent / "lib.sh").read_text()), READER]
        for index in range(1, WINDOWS + 1):
            before = unit_states(required)
            report, raw = measure(f"window-{index}", WINDOW_BOUND_S, "--window", str(WINDOW_S))
            after = unit_states(required)
            memory = {}
            for unit, state in after.items():
                try:
                    memory[unit] = rollup(int(state.get("MainPID", "0")))
                except (OSError, ValueError):
                    pass
            figures = window_figures(report, raw, before, after, memory)
            windows.append(figures)
            print(f"window {index}/{WINDOWS}: CPU {figures['backend_cpu_idle_percent']} %, writes "
                  f"{figures['resident_disk_writes_mb_day']} MB/day, PSS {figures['core_base_idle_pss_bytes']} B, "
                  f"core {memory.get(CORE)}, "
                  f"{len(figures['tolerated_reader_helpers'])} reader helper(s) tolerated, "
                  f"{len(figures['problems'])} problem(s)", flush=True)
        timings["windows"] = round(time.monotonic() - mark, 1)
        mark = time.monotonic()
        for index in range(1, RESTARTS + 1):
            time.sleep(RESTART_SETTLE_S)
            report, _ = measure(f"restart-{index}", RESTART_BOUND_S, "--window", "1", "--measure-core-restart")
            value, note = readiness_of(report)
            readiness.append(value)
            print(f"restart {index}/{RESTARTS}: ready in {value} ms" + (f" ({note})" if value is None else ""), flush=True)
        timings["restarts"] = round(time.monotonic() - mark, 1)
    except BaseException as error:  # noqa: BLE001 - recorded in the result, then re-raised
        result["error"] = repr(error)[:4000]
        write()
        raise
    write()
    for name, gate in result["gates"].items():
        print(("PASS " if gate["pass"] else "FAIL ") + f"{name}: {json.dumps({k: v for k, v in gate.items() if k != 'budget_note'})}", flush=True)
    for problem in result["problems"]:
        print("FAIL " + problem, flush=True)
    if not result["gates"]["backend_cpu_idle"]["budget_met"]:
        print(f"NOTE idle CPU {result['gates']['backend_cpu_idle']['value_percent']} % does not meet the "
              f"{CPU_BUDGET_PERCENT} % budget (not gated)", flush=True)
    if result["result"] != "pass":
        raise SystemExit("footprint gates failed: " + ", ".join(result["failed_gates"] + result["problems"]))


if __name__ == "__main__":
    main(package_version=sys.argv[3] if len(sys.argv) > 3 else "0.4.4")
