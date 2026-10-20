# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""One offline Debian-compatible release build with owned-container cleanup."""
import datetime
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import time

root = Path(__file__).resolve().parents[2]
state = Path(__file__).resolve().parent
out = state / "debian-build"
out.mkdir()
image = "sha256:af684f63adae359e95f9082cc3235bcc76208baaa9cd1a48b553784e226d5ba5"
spec = importlib.util.spec_from_file_location("a1_build_guards", root / ".cache/main-consolidation/guard.py")
guards = importlib.util.module_from_spec(spec)
spec.loader.exec_module(guards)
if subprocess.check_output(["git", "status", "--porcelain=v1"], cwd=root).strip():
    raise RuntimeError("Commit the candidate before building")
busy = [line for line in subprocess.check_output(["ps", "-eo", "pid,comm"], text=True).splitlines() if line.split()[-1] in ("cargo", "rustc", "rustdoc")]
if busy:
    raise RuntimeError("Another compiler is active")
floor = 3 * 1024**3
disk_floor = 8 * 1024**3
if guards.available() < floor or shutil.disk_usage(root).free < disk_floor:
    raise RuntimeError("Insufficient memory or disk headroom before build")
record = {"source": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=root, text=True).strip(), "image": image, "started": datetime.datetime.now(datetime.timezone.utc).isoformat(), "before": guards.snapshot(), "minimum_available_bytes": guards.available(), "minimum_disk_free_bytes": shutil.disk_usage(root).free, "memory_floor_bytes": floor, "disk_floor_bytes": disk_floor, "stop_reason": None, "one_build_job": True}
metadata = out / "record.json"
container = None
child = None

def docker(*args, check=True):
    return subprocess.run(["docker", *args], check=check, text=True, capture_output=True)

def interrupted(signum, frame):
    raise KeyboardInterrupt(f"signal {signum}")

signal.signal(signal.SIGTERM, interrupted)
signal.signal(signal.SIGINT, interrupted)
command = ["create", "--name", "limeos-a1-namespace-release", "--label", "limeos.qualification=a1-namespace-review", "--network=none", "--read-only", "--memory=2g", "--memory-swap=2g", "--pids-limit=64", "--cpus=1", "--cap-drop=ALL", "--security-opt=no-new-privileges", "--security-opt=label=disable", "--user", "1000:1000", "--volume", str(root) + ":/build", "--workdir", "/build", "--env", "CARGO_HOME=/build/.cache/p04-a1-integration/cargo-home", "--env", "CARGO_TARGET_DIR=/build/.cache/a1-namespace-review/debian-target", "--env", "CARGO_BUILD_JOBS=1", "--env", "CARGO_INCREMENTAL=0", "--env", "TMPDIR=/build/.cache/a1-namespace-review/tmp", image, "cargo", "build", "--locked", "--offline", "--release", "-p", "limeos-executor"]
record["create_command"] = ["docker", *command]
try:
    container = docker(*command).stdout.strip()
    assert re.fullmatch(r"[a-f0-9]{64}", container)
    inspect = json.loads(docker("inspect", container).stdout)[0]
    assert inspect["Image"] == image
    assert inspect["Config"]["Labels"]["limeos.qualification"] == "a1-namespace-review"
    (out / "container-before.json").write_text(json.dumps(inspect, indent=2) + "\n")
    record["container"] = container
    metadata.write_text(json.dumps(record, indent=2) + "\n")
    with (out / "output.log").open("wb") as output:
        child = subprocess.Popen(["docker", "start", "--attach", container], stdout=output, stderr=subprocess.STDOUT)
        deadline = time.monotonic() + 1200
        while child.poll() is None:
            memory = guards.available()
            disk = shutil.disk_usage(root).free
            record["minimum_available_bytes"] = min(memory, record["minimum_available_bytes"])
            record["minimum_disk_free_bytes"] = min(disk, record["minimum_disk_free_bytes"])
            if memory < floor or disk < disk_floor or time.monotonic() >= deadline:
                record["stop_reason"] = "memory floor" if memory < floor else "disk floor" if disk < disk_floor else "build deadline"
                docker("stop", "--time", "1", container)
                break
            time.sleep(0.25)
        record["attach_exit_code"] = child.wait(timeout=15)
    after = json.loads(docker("inspect", container).stdout)[0]
    (out / "container-after.json").write_text(json.dumps(after, indent=2) + "\n")
    record["exit_code"] = after["State"]["ExitCode"]
    record["oom_killed"] = after["State"]["OOMKilled"]
    assert not after["State"]["Running"]
finally:
    if container is not None:
        status = json.loads(docker("inspect", container).stdout)[0]
        assert status["Image"] == image
        assert status["Config"]["Labels"]["limeos.qualification"] == "a1-namespace-review"
        if status["State"]["Running"]:
            docker("stop", "--time", "1", container)
        if child is not None:
            child.wait(timeout=15)
        docker("rm", container)
        record["container_removed"] = docker("inspect", container, check=False).returncode != 0
    record["after"] = guards.snapshot()
    record["inputs_unchanged"] = record["before"] == record["after"]
    record["ended"] = datetime.datetime.now(datetime.timezone.utc).isoformat()
    binary = state / "debian-target/release/limeos-executor"
    if binary.is_file():
        record["binary_sha256"] = hashlib.sha256(binary.read_bytes()).hexdigest()
    metadata.write_text(json.dumps(record, indent=2) + "\n")
print(json.dumps({key: record.get(key) for key in ("source", "exit_code", "stop_reason", "oom_killed", "minimum_available_bytes", "minimum_disk_free_bytes", "container_removed", "inputs_unchanged", "binary_sha256")}))
raise SystemExit(0 if record.get("exit_code") == 0 and record["stop_reason"] is None and record["inputs_unchanged"] and record.get("container_removed") else 1)
