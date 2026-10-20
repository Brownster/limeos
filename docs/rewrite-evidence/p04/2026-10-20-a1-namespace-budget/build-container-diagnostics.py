# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Verify every frozen shared input before the offline container compilation."""
from pathlib import Path
import subprocess
import sys

manifest = Path(sys.argv[1])
with manifest.with_name("container-source-check.log").open("wb") as output:
    subprocess.run(["sha256sum", "--check", str(manifest)], stdout=output, stderr=subprocess.STDOUT, check=True)
print("SOURCE_BINDING_OK", len(manifest.read_text().splitlines()), flush=True)
subprocess.run(["cargo", "build", "--locked", "--offline", "--release", "-p", "limeos-executor"], check=True)
