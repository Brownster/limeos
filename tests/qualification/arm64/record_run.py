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
import re
import shutil
from pathlib import Path

from qualification import load_build, load_manifest, sha

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
    "installed_bytes": 154_000_000,
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
        all_pss = pss + sum(
            s["pss_kib"] for s in idle.get("optional_services", {}).values()
        )
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
                "Measured resident LimeOS services, idle PSS (MiB); assistant unavailable",
                mib(all_pss),
                BUDGETS["all_resident_limeos_pss_mib"],
                all_pss <= BUDGETS["all_resident_limeos_pss_mib"] * 1024,
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
    if window and window["seconds"] >= 600:
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
        counters = [
            v.get("process_write_bytes")
            for u, v in window["per_unit"].items()
            if u.startswith("limeos-")
        ]
        written = (
            sum(counters) if all(v is not None and v >= 0 for v in counters) else None
        )
        per_day = (
            written * 86400 / window["seconds"] / 1e6 if written is not None else None
        )
        rows.append(
            [
                "Resident writes, idle, MB/day (process write_bytes over 600 s, extrapolated)",
                round(per_day, 2) if per_day is not None else "unavailable",
                BUDGETS["resident_writes_mb_per_day"],
                per_day < BUDGETS["resident_writes_mb_per_day"]
                if per_day is not None
                else None,
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
        counters = [
            v.get("process_write_bytes")
            for u, v in dashboards["per_unit"].items()
            if u.startswith("limeos-")
        ]
        written = (
            sum(counters) if all(v is not None and v >= 0 for v in counters) else None
        )
        per_day = (
            written * 86400 / dashboards["seconds"] / 1e6
            if written is not None
            else None
        )
        rows.append(
            [
                "Resident writes with 3 dashboards, MB/day (process write_bytes over 300 s, extrapolated)",
                round(per_day, 2) if per_day is not None else "unavailable",
                BUDGETS["resident_writes_mb_per_day"],
                per_day < BUDGETS["resident_writes_mb_per_day"]
                if per_day is not None
                else None,
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
    installed = install.get("observations", {}).get("installed_bytes")
    if installed is not None:
        rows.append(
            [
                "Installed package bytes including UI (MB)",
                round(installed / 1e6, 2),
                BUDGETS["installed_bytes"] / 1e6,
                installed < BUDGETS["installed_bytes"],
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
    parser.add_argument("--extra-result", type=Path)
    parser.add_argument("--build-result", type=Path, required=True)
    parser.add_argument("--source-manifest", type=Path, required=True)
    parser.add_argument("--suite-result", type=Path, action="append", default=[])
    args = parser.parse_args()
    if not re.fullmatch(r"[a-z0-9][a-z0-9-]{0,100}", args.run):
        parser.error("run must be a new simple directory name")
    source = load_manifest(args.source_manifest)
    build = load_build(args.build_result)
    if source["identity"] != build["identity"]:
        parser.error("source and native build identity differ")
    if sha(args.source_manifest) != build["source_manifest_sha256"]:
        parser.error("source manifest bytes differ from the native build input")
    for field in ("runtime_source_sha256", "frontend_dist_sha256", "fixtures_sha256"):
        if source[field] != build[field]:
            parser.error(f"source and build {field} differ")
    results = {}
    for path in [args.install_result, args.extra_result, *args.suite_result]:
        if path is None:
            continue
        result = json.loads(path.read_text())
        if result["identity"] != build["identity"] or result[
            "build_result_sha256"
        ] != sha(args.build_result):
            parser.error(f"measurement identity differs from native build: {path}")
        results[str(path)] = result
    out = ROOT / "docs/rewrite-evidence/arm64" / args.run
    if out.exists():
        parser.error("run exists; historical evidence is immutable")
    copies = [
        (args.source_manifest, "source-manifest.json"),
        (args.build_result, "build/build-result.json"),
    ]
    copies.extend(
        (Path(src), dest)
        for src, dest in (item.split("=", 1) for item in args.artifact)
    )
    copies.extend((Path(path), "results/" + Path(path).name) for path in results)
    destinations = set()
    for src, dest in copies:
        target = (out / dest).resolve()
        if (
            not src.exists()
            or not target.is_relative_to(out.resolve())
            or target in destinations
            or Path(dest).is_absolute()
            or target == out.resolve()
            or any(
                target.is_relative_to(other) or other.is_relative_to(target)
                for other in destinations
            )
        ):
            parser.error(f"invalid or duplicate artifact: {src}={dest}")
        destinations.add(target)
    out.mkdir(parents=True)
    for source, destination in copies:
        target = out / destination
        target.parent.mkdir(parents=True, exist_ok=True)
        if Path(source).is_dir():
            shutil.copytree(source, target, dirs_exist_ok=True)
        else:
            shutil.copy2(source, target)
    if args.install_result:
        rows = budget_table(results[str(args.install_result)])
        if args.extra_result:
            extra = results[str(args.extra_result)]
            for key in ("limeosctl_status", "executor_storage_inventory"):
                measurement = extra[key]
                rows.append(
                    [
                        key + " p95 (ms)",
                        measurement["p95"],
                        BUDGETS["executor_and_ctl_start_ms"],
                        measurement["successful"]
                        and measurement["p95"] < BUDGETS["executor_and_ctl_start_ms"],
                    ]
                )
        (out / "budgets.json").write_text(json.dumps(rows, indent=2) + "\n")
        print("| Measurement | Measured | Budget | Within |\n|---|---|---|---|")
        for name, measured, budget, ok in rows:
            print(
                f"| {name} | {measured} | {budget} | {'-' if ok is None else ('yes' if ok else 'NO')} |"
            )
    reached = {r.get("suite", r.get("kind")) for r in results.values()}
    untested = sorted({"install", "extra", "storage", "container", "upgrade"} - reached)
    if not any(
        r.get("suite") == "upgrade"
        and r.get("previous", {}).get("authority_schema") == 7
        for r in results.values()
    ):
        untested.append("genuine schema 7 ARM64 upgrade to schema 8")
    (out / "qualification-status.json").write_text(
        json.dumps(
            {
                "identity": build["identity"],
                "untested_gates": untested,
                "result_summaries": {
                    p: r.get("summary", "incomplete") for p, r in results.items()
                },
                "limitations": [
                    "Native KVM guest evidence; bare metal and comparable Python baseline are separate untested claims",
                    "Daily write values extrapolate only the recorded interval",
                    "Eight registered storage disks and assistant-inclusive workload were not measured by the footprint fixture",
                ],
            },
            indent=2,
        )
        + "\n"
    )
    manifest = {
        str(p.relative_to(out)): hashlib.sha256(p.read_bytes()).hexdigest()
        for p in sorted(out.rglob("*"))
        if p.is_file() and p.name != "evidence-sha256.json"
    }
    (out / "evidence-sha256.json").write_text(json.dumps(manifest, indent=2) + "\n")


if __name__ == "__main__":
    main()
