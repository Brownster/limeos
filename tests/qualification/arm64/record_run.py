#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Assemble one native ARM64 run's raw artifacts and compute its budget table.

Copies results into docs/rewrite-evidence/arm64/<run>/, hashes every file it
writes, and prints the measured values against the architecture budgets. The
numbers in the report come from this output, not from hand transcription.
"""

import argparse
import hashlib
import json
import shutil
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]

# Architecture section 10 targets (provisional P01 budgets).
BUDGETS = {
    "core_and_executors_pss_mib": 30,
    "all_resident_limeos_pss_mib": 60,
    "idle_cpu_percent_of_one_core": 0.1,
    "executor_and_ctl_start_ms": 20,
    "core_ready_ms": 1000,
    "cached_read_p95_ms": 20,
    "resident_writes_mb_per_day": 20,
}


def check(by_name, name):
    entry = by_name.get(name)
    return entry["observation"] if entry and entry["result"] == "pass" else None


def budget_table(install):
    by_name = {c["name"]: c for c in install["checks"]}
    rows = []
    idle = check(by_name, "idle footprint after authentication (no subscribers)")
    final = check(by_name, "final footprint after workload and shadow removal")
    window = check(
        by_name, "idle CPU and bytes written over 600 seconds, no subscribers"
    )
    dashboards = check(
        by_name, "CPU and bytes written over 300 seconds with 3 open dashboard streams"
    )
    latency = check(
        by_name, "authenticated read latency, 3 concurrent clients x 200 requests"
    )
    restarts = check(by_name, "core restart readiness distribution (20 restarts)")
    cold = check(by_name, "full stack cold start readiness (5 starts)")

    def mib(kib):
        return round(kib / 1024, 2)

    if idle:
        pss = idle["limeos_sum"]["pss_kib"]
        rows.append(
            [
                "Core and base executors, idle PSS (MiB)",
                mib(pss),
                BUDGETS["core_and_executors_pss_mib"],
                pss <= BUDGETS["core_and_executors_pss_mib"] * 1024,
            ]
        )
        rows.append(
            [
                "All resident LimeOS processes, idle PSS (MiB); assistant not present in 0.4.2",
                mib(pss),
                BUDGETS["all_resident_limeos_pss_mib"],
                pss <= BUDGETS["all_resident_limeos_pss_mib"] * 1024,
            ]
        )
        rows.append(
            [
                "Steady-state swap (KiB)",
                idle["limeos_sum"]["swap_kib"],
                0,
                idle["limeos_sum"]["swap_kib"] == 0,
            ]
        )
    if final:
        pss = final["limeos_sum"]["pss_kib"]
        rows.append(
            [
                "Core and executors PSS after workload (MiB)",
                mib(pss),
                BUDGETS["core_and_executors_pss_mib"],
                pss <= BUDGETS["core_and_executors_pss_mib"] * 1024,
            ]
        )
    if window:
        cpu = window["limeos_cpu_percent_of_one_core"]
        rows.append(
            [
                "Idle CPU, 600 s, % of one core",
                cpu,
                BUDGETS["idle_cpu_percent_of_one_core"],
                cpu < BUDGETS["idle_cpu_percent_of_one_core"],
            ]
        )
        # cgroup io.stat reported no wbytes for these units in the guest; use the
        # per-process write_bytes counters, which do attribute the writes.
        written = sum(
            v.get("process_write_bytes", 0)
            for u, v in window["per_unit"].items()
            if u.startswith("limeos-")
        )
        per_day = written * 86400 / window["seconds"] / 1e6
        rows.append(
            [
                "Resident writes, idle, MB/day (process write_bytes over 600 s, extrapolated)",
                round(per_day, 2),
                BUDGETS["resident_writes_mb_per_day"],
                per_day < BUDGETS["resident_writes_mb_per_day"],
            ]
        )
    if dashboards:
        rows.append(
            [
                "CPU with 3 dashboard streams, % of one core (no budget)",
                dashboards["limeos_cpu_percent_of_one_core"],
                "-",
                None,
            ]
        )
        written = sum(
            v.get("process_write_bytes", 0)
            for u, v in dashboards["per_unit"].items()
            if u.startswith("limeos-")
        )
        per_day = written * 86400 / dashboards["seconds"] / 1e6
        rows.append(
            [
                "Resident writes with 3 dashboards, MB/day (process write_bytes over 300 s, extrapolated)",
                round(per_day, 2),
                BUDGETS["resident_writes_mb_per_day"],
                per_day < BUDGETS["resident_writes_mb_per_day"],
            ]
        )
    if restarts:
        rows.append(
            [
                "Core restart to ready, p95 / max (ms)",
                f"{restarts['p95']} / {restarts['max']}",
                BUDGETS["core_ready_ms"],
                restarts["max"] < BUDGETS["core_ready_ms"],
            ]
        )
    if cold:
        rows.append(
            [
                "Full stack cold start to ready, max (ms)",
                cold["max"],
                BUDGETS["core_ready_ms"],
                cold["max"] < BUDGETS["core_ready_ms"],
            ]
        )
    if latency:
        p95 = latency["sequential_cached_overview_ms"]["p95"]
        rows.append(
            [
                "Cached overview, sequential, p95 (ms)",
                p95,
                BUDGETS["cached_read_p95_ms"],
                p95 < BUDGETS["cached_read_p95_ms"],
            ]
        )
        p95 = latency["all_ms"]["p95"]
        rows.append(
            [
                "Authenticated reads, 3 concurrent clients, p95 (ms)",
                p95,
                BUDGETS["cached_read_p95_ms"],
                p95 < BUDGETS["cached_read_p95_ms"],
            ]
        )
    return rows


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--run", required=True, help="Evidence directory name")
    parser.add_argument(
        "--artifact",
        action="append",
        default=[],
        help="SRC=DEST relative to the run directory",
    )
    parser.add_argument("--install-result", type=Path)
    args = parser.parse_args()
    out = ROOT / "docs/rewrite-evidence/arm64" / args.run
    out.mkdir(parents=True, exist_ok=True)
    for item in args.artifact:
        source, destination = item.split("=", 1)
        target = out / destination
        target.parent.mkdir(parents=True, exist_ok=True)
        if Path(source).is_dir():
            shutil.copytree(source, target, dirs_exist_ok=True)
        else:
            shutil.copy2(source, target)
    if args.install_result:
        rows = budget_table(json.loads(args.install_result.read_text()))
        (out / "budgets.json").write_text(json.dumps(rows, indent=2) + "\n")
        print("| Measurement | Measured | Budget | Within |\n|---|---|---|---|")
        for name, measured, budget, ok in rows:
            print(
                f"| {name} | {measured} | {budget} | {'-' if ok is None else ('yes' if ok else 'NO')} |"
            )
    manifest = {
        str(p.relative_to(out)): hashlib.sha256(p.read_bytes()).hexdigest()
        for p in sorted(out.rglob("*"))
        if p.is_file() and p.name != "evidence-sha256.json"
    }
    (out / "evidence-sha256.json").write_text(json.dumps(manifest, indent=2) + "\n")


if __name__ == "__main__":
    main()
