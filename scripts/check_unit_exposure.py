#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Fail if a packaged systemd unit's exposure score rises above its recorded ceiling.

Scores come from `systemd-analyze security --offline=true` (systemd 250 or newer),
which reads the unit file alone: no install, no root, no running manager. A unit
without a recorded ceiling fails too, so a new unit is scored deliberately.

Ceilings are tenths of systemd's 0-10 exposure level (its --threshold scale) and
were recorded with systemd 258. Lower one when a unit is tightened; raising one
needs the reason in the unit file next to the setting that costs it.
"""

import argparse
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CEILINGS = {
    "limeos-containerd.service": 21,
    "limeos-core.service": 25,
    "limeos-storaged.service": 4,
    "limeos-storage-mounts.service": 46,
    "limeos-storage-reader.service": 35,
    "limeos-storage-ready.service": 32,
    "limeos-storage-targets.service": 36,
}
OVERALL = re.compile(r"Overall exposure level for \S+: (\d+)\.(\d)")


def score(unit: Path, ceiling: int) -> tuple[int | None, bool, str]:
    """Return (exposure in tenths, within ceiling, output) for one unit file."""
    result = subprocess.run(
        [
            "systemd-analyze",
            "security",
            "--offline=true",
            f"--threshold={ceiling}",
            "--no-pager",
            str(unit),
        ],
        capture_output=True,
        text=True,
        timeout=60,
        check=False,
    )
    output = result.stdout + result.stderr
    found = OVERALL.search(output)
    exposure = int(found[1]) * 10 + int(found[2]) if found else None
    return exposure, result.returncode == 0 and exposure is not None, output


def check(units: Path, ceilings: dict[str, int]) -> list[str]:
    problems = []
    present = sorted(path.name for path in units.glob("*.service"))
    for name in sorted(set(ceilings) - set(present)):
        problems.append(f"{name}: ceiling recorded for a unit that does not exist")
    for name in present:
        if name not in ceilings:
            problems.append(f"{name}: no exposure ceiling recorded in {Path(__file__).name}")
            continue
        exposure, within, output = score(units / name, ceilings[name])
        if exposure is None:
            problems.append(f"{name}: systemd-analyze gave no score:\n{output.strip()}")
            continue
        print(f"{name}: exposure {exposure / 10:.1f} (ceiling {ceilings[name] / 10:.1f})")
        if not within:
            problems.append(
                f"{name}: exposure {exposure / 10:.1f} is above its ceiling "
                f"{ceilings[name] / 10:.1f}"
            )
        elif exposure < ceilings[name]:
            print(f"  {name} scores below its ceiling; lower the ceiling to keep the gain.")
    return problems


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--units", type=Path, default=ROOT / "packaging/systemd")
    args = parser.parse_args()
    problems = check(args.units, CEILINGS)
    for problem in problems:
        print(problem, file=sys.stderr)
    if not problems:
        print("Unit exposure scores are within their recorded ceilings.")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
