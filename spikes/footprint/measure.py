#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Measure RW-005 on Linux. Only stdlib is needed on the reference host.

Local: uv run spikes/footprint/measure.py --binary target/release/limeos-footprint --fixture --idle-seconds 30
Reference host: python3 measure.py --binary ./limeos-footprint --output result.json
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
import platform
import selectors
import socketserver
import subprocess
import sys
import tempfile
import threading
import time
from collections.abc import Iterator
from concurrent.futures import ThreadPoolExecutor
from contextlib import contextmanager
from datetime import datetime, timezone
from http.client import HTTPConnection
from http.server import BaseHTTPRequestHandler
from pathlib import Path


def distribution(values: list[float]) -> dict[str, float]:
    ordered = sorted(values)
    return {
        "min": ordered[0],
        "p50": ordered[math.ceil(len(ordered) * 0.50) - 1],
        "p95": ordered[math.ceil(len(ordered) * 0.95) - 1],
        "max": ordered[-1],
    }


def read_counters(pid: int) -> dict:
    proc = Path(f"/proc/{pid}")
    memory = {}
    for line in (proc / "smaps_rollup").read_text().splitlines():
        fields = line.split()
        if fields[0] in {"Pss:", "Rss:", "Swap:"}:
            memory[fields[0].removesuffix(":").lower() + "_kib"] = int(fields[1])
    io = {}
    for line in (proc / "io").read_text().splitlines():
        name, value = line.split(":")
        io[name] = int(value)
    # The parenthesized comm can contain spaces and ')'. Fields after it begin at #3.
    stat = (proc / "stat").read_text().rsplit(")", 1)[1].split()
    return {
        **memory,
        "cpu_ticks": int(stat[11]) + int(stat[12]),
        "threads": int(stat[17]),
        "write_bytes": io["write_bytes"],
        "cancelled_write_bytes": io["cancelled_write_bytes"],
    }


def request(address: str, path: str, expected_pid: int | None = None) -> dict:
    host, port = address.rsplit(":", 1)
    connection = HTTPConnection(host, int(port), timeout=5)
    try:
        connection.request("GET", path)
        response = connection.getresponse()
        body = response.read(1024 * 1024 + 1)
        if response.status != 200 or len(body) > 1024 * 1024:
            raise RuntimeError(f"invalid {path} response: HTTP {response.status}")
        data = json.loads(body)
        if expected_pid is not None and data.get("pid") != expected_pid:
            raise RuntimeError("readiness responded from a different process")
        return data
    finally:
        connection.close()


@contextmanager
def start_probe(binary: Path, state: Path, docker_socket: str) -> Iterator[tuple]:
    state.mkdir(mode=0o700)
    environment = {
        **os.environ,
        "LIMEOS_SPIKE_STATE_DIR": str(state),
        "LIMEOS_SPIKE_LISTEN": "127.0.0.1:0",
        "LIMEOS_SPIKE_DOCKER_SOCKET": docker_socket,
    }
    with tempfile.TemporaryFile(mode="w+b") as errors:
        started = time.monotonic()
        process = subprocess.Popen(
            [str(binary)], env=environment, stdout=subprocess.PIPE, stderr=errors
        )
        try:
            with selectors.DefaultSelector() as selector:
                selector.register(process.stdout, selectors.EVENT_READ)
                if not selector.select(timeout=5):
                    raise RuntimeError(
                        "probe did not report readiness within five seconds"
                    )
            line = process.stdout.readline(64 * 1024)
            if not line:
                errors.seek(0)
                raise RuntimeError(errors.read(64 * 1024).decode(errors="replace"))
            ready = json.loads(line)
            if ready.get("event") != "ready" or ready.get("pid") != process.pid:
                raise RuntimeError("invalid startup event")
            request(ready["listen"], "/health/ready", process.pid)
            startup_ms = (time.monotonic() - started) * 1000
            # Readiness requires a functioning DB worker, not just a listening socket.
            overview = request(ready["listen"], "/api/v1/overview")
            if overview["sqlite_rows"] != 1:
                raise RuntimeError("SQLite smoke read failed")
            yield process, ready, startup_ms
            if process.poll() is not None:
                raise RuntimeError("probe exited during measurement")
        finally:
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=5)
            if process.stdout:
                process.stdout.close()


@contextmanager
def fixture_socket(root: Path) -> Iterator[str]:
    """Exercise the real Docker HTTP client locally without needing Docker access."""
    body = json.dumps(
        [{"Id": f"fixture-{i}", "State": "running"} for i in range(18)]
        + [{"Id": "fixture-stopped", "State": "exited"}]
    ).encode()

    class Handler(BaseHTTPRequestHandler):
        def do_GET(self) -> None:
            if self.path != "/containers/json?all=1":
                self.send_error(404)
                return
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.send_header("Connection", "close")
            self.end_headers()
            self.wfile.write(body)

        def log_message(self, *_args) -> None:
            pass

    path = str(root / "docker.sock")
    with socketserver.UnixStreamServer(path, Handler) as server:
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            yield path
        finally:
            server.shutdown()
            thread.join(timeout=5)


def latency_run(address: str, count: int) -> list[float]:
    result = []
    for _ in range(count):
        started = time.monotonic()
        data = request(address, "/api/v1/overview")
        if data["sqlite_rows"] != 1:
            raise RuntimeError("overview returned an invalid database observation")
        result.append((time.monotonic() - started) * 1000)
    return result


def measure(args: argparse.Namespace, root: Path, docker_socket: str) -> dict:
    startup = []
    for index in range(args.startup_runs - 1):
        with start_probe(
            args.binary, root / f"startup-{index}", docker_socket
        ) as probe:
            startup.append(probe[2])
    with start_probe(args.binary, root / "measured", docker_socket) as probe:
        process, ready, startup_ms = probe
        startup.append(startup_ms)
        address = ready["listen"]
        latency_run(address, 20)
        before_load = read_counters(process.pid)
        load_started = time.monotonic()
        sequential = latency_run(address, args.latency_requests)
        with ThreadPoolExecutor(max_workers=args.clients) as executor:
            concurrent = list(
                executor.map(
                    lambda _index: latency_run(address, args.latency_requests),
                    range(args.clients),
                )
            )
        load_seconds = time.monotonic() - load_started
        after_load = read_counters(process.pid)
        idle_started = time.monotonic()
        idle_before = read_counters(process.pid)
        samples = [{"elapsed_seconds": 0.0, **idle_before}]
        next_update = 60
        while time.monotonic() - idle_started < args.idle_seconds:
            time.sleep(
                max(0, min(1, args.idle_seconds - (time.monotonic() - idle_started)))
            )
            elapsed = time.monotonic() - idle_started
            samples.append({"elapsed_seconds": elapsed, **read_counters(process.pid)})
            if elapsed >= next_update:
                print(
                    f"idle measurement: {elapsed:.0f}/{args.idle_seconds:g} seconds",
                    file=sys.stderr,
                    flush=True,
                )
                next_update += 60
        idle_elapsed = time.monotonic() - idle_started
        idle_after = read_counters(process.pid)
        ticks_per_second = os.sysconf("SC_CLK_TCK")
        idle_cpu = (
            idle_after["cpu_ticks"] - idle_before["cpu_ticks"]
        ) / ticks_per_second
        idle_writes = idle_after["write_bytes"] - idle_before["write_bytes"]
        model_path = Path("/sys/firmware/devicetree/base/model")
        model = model_path.read_text().rstrip("\0\n") if model_path.exists() else None
        architecture = platform.machine()
        qualified_host = (
            architecture == "aarch64"
            and model is not None
            and "Raspberry Pi 5" in model
            and not args.fixture
        )
        pss_max_mib = max(sample["pss_kib"] for sample in samples) / 1024
        swap_max_mib = max(sample["swap_kib"] for sample in samples) / 1024
        startup_pass = max(startup) < 1000
        memory_pass = pss_max_mib < 15 and swap_max_mib == 0
        adequate_run = args.startup_runs >= 10 and idle_elapsed >= 600
        return {
            "schema_version": 1,
            "measured_utc": datetime.now(timezone.utc).isoformat(),
            "host": {
                "architecture": architecture,
                "model": model,
                "kernel": platform.release(),
                "logical_cpus": os.cpu_count(),
                "os_release": Path("/etc/os-release").read_text(),
                "clock_ticks_per_second": ticks_per_second,
            },
            "build": {
                "binary_sha256": hashlib.sha256(args.binary.read_bytes()).hexdigest(),
                "binary_bytes": args.binary.stat().st_size,
            },
            "workload": {
                "docker_source": "fixture" if args.fixture else "live_unix_socket",
                "inventory": ready["inventory"],
                "runtime_workers": ready["runtime_workers"],
                "database_workers": ready["database_workers"],
                "startup_runs": args.startup_runs,
                "startup_conditions": "warm OS page cache; fresh SQLite database per start",
                "latency_requests_per_client": args.latency_requests,
                "concurrent_clients": args.clients,
                "http_connections": "one new loopback HTTP/1 connection per request",
                "tls": "Rustls client initialized; no external TLS handshake or model request",
            },
            "startup_ms": {"samples": startup, **distribution(startup)},
            "latency_ms": {
                "sequential": distribution(sequential),
                "concurrent": distribution(
                    [item for client in concurrent for item in client]
                ),
            },
            "load": {
                "elapsed_seconds": load_seconds,
                "cpu_seconds": (after_load["cpu_ticks"] - before_load["cpu_ticks"])
                / ticks_per_second,
                "write_bytes": after_load["write_bytes"] - before_load["write_bytes"],
                "memory_after": after_load,
            },
            "idle": {
                "elapsed_seconds": idle_elapsed,
                "cpu_seconds": idle_cpu,
                "cpu_percent_one_core": idle_cpu / idle_elapsed * 100,
                "pss_max_mib": pss_max_mib,
                "rss_max_mib": max(sample["rss_kib"] for sample in samples) / 1024,
                "swap_max_mib": swap_max_mib,
                "write_bytes": idle_writes,
                "cancelled_write_bytes": idle_after["cancelled_write_bytes"]
                - idle_before["cancelled_write_bytes"],
                "samples": samples,
            },
            "gates": {
                "reference_pi5_live_docker": qualified_host,
                "readiness_all_under_1_second": startup_pass,
                "idle_pss_under_15_mib_without_swap": memory_pass,
                "ten_starts_and_ten_minute_idle": adequate_run,
                "rw005_footprint_pass": qualified_host
                and startup_pass
                and memory_pass
                and adequate_run,
                "idle_cpu_under_0_1_percent": adequate_run
                and idle_cpu / idle_elapsed * 100 < 0.1,
            },
        }


def main() -> int:
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    parser.add_argument(
        "--binary", required=True, type=Path, help="release binary to measure"
    )
    parser.add_argument(
        "--output", type=Path, help="new JSON result file; defaults to stdout"
    )
    parser.add_argument("--idle-seconds", type=float, default=600)
    parser.add_argument("--startup-runs", type=int, default=10)
    parser.add_argument("--latency-requests", type=int, default=300)
    parser.add_argument("--clients", type=int, default=3)
    parser.add_argument("--docker-socket", default="/var/run/docker.sock")
    parser.add_argument(
        "--fixture",
        action="store_true",
        help="fake Docker for harness validation; never qualifies the Pi gate",
    )
    args = parser.parse_args()
    if platform.system() != "Linux":
        parser.error("measurements require Linux /proc")
    if not args.binary.is_file():
        parser.error("--binary must be an existing release executable")
    if (
        not math.isfinite(args.idle_seconds)
        or args.idle_seconds <= 0
        or min(args.startup_runs, args.latency_requests, args.clients) < 1
    ):
        parser.error("measurement counts and duration must be positive and finite")
    if args.output and args.output.exists():
        parser.error("--output already exists; choose a new evidence file")
    args.binary = args.binary.resolve()
    try:
        with tempfile.TemporaryDirectory(prefix="limeos-rw005-") as directory:
            root = Path(directory)
            if args.fixture:
                with fixture_socket(root) as docker_socket:
                    result = measure(args, root, docker_socket)
            else:
                result = measure(args, root, args.docker_socket)
        serialized = json.dumps(result, indent=2) + "\n"
        if args.output:
            args.output.parent.mkdir(parents=True, exist_ok=True)
            with args.output.open("x") as output:
                output.write(serialized)
            print(
                f"saved {args.output}; gates: {json.dumps(result['gates'])}",
                file=sys.stderr,
            )
        else:
            print(serialized, end="")
        return 0
    except (OSError, RuntimeError, ValueError, subprocess.SubprocessError) as error:
        print(f"measurement failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
