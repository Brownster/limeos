#!/usr/bin/env python3
"""Short-lived command timings and the optional root storage reader's footprint.

Runs inside the disposable storage guest after the P04 suite has configured
storage policy:  extra_guest.py OUTPUT_JSON
"""

import json
import os
import platform
import socket
import subprocess
import sys
import time
from pathlib import Path

OUTPUT = Path(sys.argv[1])
LIMEOS = Path("/usr/lib/limeos")


def run(*args, check=True):
    result = subprocess.run(
        args, text=True, capture_output=True, timeout=60, check=False
    )
    if check and result.returncode != 0:
        raise RuntimeError(f"{args}: {result.returncode} {result.stderr[-1000:]}")
    return result


def timed(command, count):
    values = []
    codes = set()
    for _ in range(count):
        started = time.monotonic()
        codes.add(
            subprocess.run(
                command, capture_output=True, timeout=60, check=False
            ).returncode
        )
        values.append(round((time.monotonic() - started) * 1000, 3))
    values.sort()
    return {
        "command": " ".join(command),
        "exit_codes": sorted(codes),
        "n": count,
        "min": values[0],
        "p50": values[len(values) // 2],
        "p95": values[min(count - 1, round(0.95 * (count - 1)))],
        "max": values[-1],
    }


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


def main():
    if (
        os.getuid() != 0
        or socket.gethostname() != "limeos-p01-test"
        or platform.machine() != "aarch64"
    ):
        raise SystemExit("Disposable native aarch64 guest required")
    result = {
        "limeosctl_status": timed([str(LIMEOS / "limeosctl"), "status"], 20),
        "executor_storage_inventory": timed(
            [str(LIMEOS / "limeos-executor"), "storage-inventory"], 10
        ),
    }
    run("systemctl", "reset-failed", "limeos-storage-reader", check=False)
    started = time.monotonic()
    start = run("systemctl", "start", "limeos-storage-reader", check=False)
    reader = {
        "start_exit": start.returncode,
        "start_ms": round((time.monotonic() - started) * 1000, 2),
    }
    time.sleep(2)
    pid = int(
        run(
            "systemctl", "show", "limeos-storage-reader", "-p", "MainPID", "--value"
        ).stdout
    )
    reader["active"] = run(
        "systemctl", "is-active", "limeos-storage-reader", check=False
    ).stdout.strip()
    if pid:
        reader["memory"] = smaps(pid)
        status = Path(f"/proc/{pid}/status").read_text()
        reader["uid_and_caps"] = [
            l
            for l in status.splitlines()
            if l.split(":")[0] in ("Uid", "CapEff", "CapBnd", "NoNewPrivs")
        ]
    reader["properties"] = run(
        "systemctl",
        "show",
        "limeos-storage-reader",
        "-p",
        "User",
        "-p",
        "CapabilityBoundingSet",
        "-p",
        "MemoryMax",
        "-p",
        "ExecStart",
    ).stdout
    run("systemctl", "stop", "limeos-storage-reader", check=False)
    result["storage_reader"] = reader
    OUTPUT.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result, indent=1))


if __name__ == "__main__":
    main()
