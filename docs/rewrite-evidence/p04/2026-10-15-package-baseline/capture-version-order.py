#!/usr/bin/env -S uv run --offline
# /// script
# dependencies = []
# ///
"""Capture a local read-only dpkg comparison oracle for bounded versions."""
import json
import subprocess
from pathlib import Path

root = Path(__file__).resolve().parents[1]
versions = [
    "0", "00", "0.0", "1", "1-0", "1-00", "1.0", "1.00", "1.0-1",
    "1.0-2", "1.0-10", "1.0~rc1", "1.0~~", "1.0~", "1.0a", "1.0A",
    "1.0+", "1.0.0", "1.0-1+b1", "1.0-1~deb12u1", "1.0-1+deb12u1",
    "1.0-1-1", "0:1.0", "1:0", "2:0", "0002:0", "999999999:0",
    "2.1.207-1", "2.1.207-2", "2.1.210-1", "24", "25~beta1",
    "9" * 110, "1." + "0" * 105 + "2", "1." + "0" * 105 + "3",
]
matrix = []
for left in versions:
    row = []
    for right in versions:
        lt = subprocess.run(["dpkg", "--compare-versions", left, "lt", right], check=False)
        assert lt.returncode in (0, 1)
        if lt.returncode == 0:
            row.append(-1)
        else:
            eq = subprocess.run(["dpkg", "--compare-versions", left, "eq", right], check=False)
            assert eq.returncode in (0, 1)
            row.append(0 if eq.returncode == 0 else 1)
    matrix.append(row)
path = root / "tests/fixtures/deployment/packages/version-order.json"
path.write_text(json.dumps({
    "oracle": subprocess.check_output(["dpkg", "--version"], text=True).splitlines()[0],
    "versions": versions,
    "ordering": matrix,
}, indent=2) + "\n")
print(f"Captured {len(versions) ** 2} dpkg comparisons in {path}")
