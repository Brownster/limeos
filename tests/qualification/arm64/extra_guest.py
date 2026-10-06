#!/usr/bin/env python3
"""Short-lived command timings and the optional root storage reader's footprint.

Runs inside the disposable storage guest after the P04 suite has configured
storage policy:  extra_guest.py OUTPUT_JSON
"""

import argparse
import json
import pwd
import subprocess
import time
from pathlib import Path

from qualification import guest_guard, installed_binaries, load_build, sha

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
        "samples_ms": values,
        "successful": codes == {0},
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
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--expected", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    guest_guard()
    if args.output.exists():
        parser.error("output exists; select a new result file")
    expected = load_build(args.expected)
    result = {
        "kind": "extra",
        "identity": expected["identity"],
        "build_result_sha256": sha(args.expected),
        "installed_sha256": installed_binaries(expected),
        "limeosctl_status": timed([str(LIMEOS / "limeosctl"), "status"], 20),
        "executor_storage_inventory": timed(
            [str(LIMEOS / "limeos-executor"), "storage-inventory"], 10
        ),
    }
    policy = Path("/etc/limeos/system-policy/storage-targets.json")
    original = policy.read_bytes() if policy.exists() else None
    # A closed no-effect policy lets the optional service start without granting
    # directory preparation authority. Restore the fixture's policy afterwards.
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
    result["optional_services"] = {}
    try:
        for unit in ("limeos-storage-reader", "limeos-storage-targets"):
            run("systemctl", "reset-failed", unit, check=False)
            began = time.monotonic()
            start = run("systemctl", "start", unit, check=False)
            observed = {
                "start_exit": start.returncode,
                "start_ms": round((time.monotonic() - began) * 1000, 2),
            }
            result["optional_services"][unit] = observed
            time.sleep(2)
            pid = int(run("systemctl", "show", unit, "-p", "MainPID", "--value").stdout)
            observed["active"] = run(
                "systemctl", "is-active", unit, check=False
            ).stdout.strip()
            observed["properties"] = run(
                "systemctl",
                "show",
                unit,
                "-p",
                "User",
                "-p",
                "CapabilityBoundingSet",
                "-p",
                "MemoryMax",
                "-p",
                "ExecStart",
            ).stdout
            assert start.returncode == 0 and pid and observed["active"] == "active", (
                observed
            )
            observed["memory"] = smaps(pid)
            status = Path(f"/proc/{pid}/status").read_text()
            observed["uid_and_caps"] = {
                line.split(":")[0]: line.split(":", 1)[1].strip()
                for line in status.splitlines()
                if line.split(":")[0]
                in ("Uid", "CapEff", "CapBnd", "CapAmb", "NoNewPrivs")
            }
            caps = observed["uid_and_caps"]
            assert caps["Uid"].split()[0] == "0" and caps["NoNewPrivs"] == "1", caps
            assert int(caps["CapBnd"], 16) == (1 if unit.endswith("targets") else 0), (
                caps
            )
            assert int(caps["CapEff"], 16) == (1 if unit.endswith("targets") else 0), (
                caps
            )
            run("systemctl", "stop", unit)
        assert all(
            result[k]["successful"]
            for k in ("limeosctl_status", "executor_storage_inventory")
        ), result
        result["summary"] = "pass"
    except Exception as error:
        result.update(summary="fail", error=repr(error))
        raise
    finally:
        for unit in ("limeos-storage-reader", "limeos-storage-targets"):
            run("systemctl", "stop", unit, check=False)
        if original is not None:
            policy.write_bytes(original)
        else:
            policy.unlink()
        args.output.write_text(json.dumps(result, indent=2) + "\n")


if __name__ == "__main__":
    main()
