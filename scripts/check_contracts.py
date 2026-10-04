#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"Regenerate contracts and fail on contract drift."

import subprocess
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
with tempfile.TemporaryDirectory(prefix="limeos-contracts-") as tmp:
    subprocess.run(
        ["cargo", "run", "--locked", "-p", "limeos-schema", "--", tmp],
        cwd=ROOT,
        check=True,
    )
    for actual in Path(tmp).iterdir():
        expected = ROOT / "contracts/generated" / actual.name
        assert expected.is_file() and expected.read_bytes() == actual.read_bytes(), (
            f"contract drift: {actual.name}"
        )
print("Generated schemas and TypeScript match Rust contracts.")
