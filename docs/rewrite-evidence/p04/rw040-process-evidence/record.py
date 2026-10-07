#!/usr/bin/env uv run python3
# /// script
# dependencies = []
# ///
"""Record one local command, without a shell, into a new evidence log.

Usage: uv run --offline record.py NAME -- COMMAND ARG...
Logs and metadata are exclusive-created: failed attempts are never overwritten.
"""
import datetime as dt
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import time


def main() -> int:
    name, separator, *command = sys.argv[1:]
    if separator != "--" or not command or not name.replace("-", "").isalnum():
        raise ValueError("expected NAME -- COMMAND ARG...")
    directory = Path(__file__).resolve().parent / "logs"
    directory.mkdir(exist_ok=True)
    log = directory / f"{name}.log"
    started = dt.datetime.now(dt.timezone.utc).isoformat()
    clock = time.monotonic()
    with log.open("xb") as stream:
        result = subprocess.run(command, stdout=stream, stderr=subprocess.STDOUT, check=False)
    metadata = {
        "command": command, "cwd": str(Path.cwd()), "started_utc": started,
        "elapsed_seconds": round(time.monotonic() - clock, 6),
        "exit_code": result.returncode,
        "sha256": hashlib.sha256(log.read_bytes()).hexdigest(),
        "source_commit": subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip(),
        "source_sha256": {
            str(path): hashlib.sha256(path.read_bytes()).hexdigest()
            for path in [
                Path("Cargo.lock"), Path("Cargo.toml"),
                Path("crates/executor-storage/Cargo.toml"),
                Path("crates/executor-storage/src/lib.rs"),
                Path("crates/executor-storage/src/processes.rs"),
                Path("crates/executor-storage/src/processes/tests.rs"),
                *sorted(Path("tests/fixtures/container-process-evidence").glob("*.txt")),
            ]
        },
    }
    with log.with_suffix(".json").open("x") as stream:
        json.dump(metadata, stream, indent=2)
        stream.write("\n")
    print(log.read_text(errors="replace"), end="")
    print(json.dumps(metadata, sort_keys=True))
    return result.returncode


if __name__ == "__main__":
    raise SystemExit(main())
