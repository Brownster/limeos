#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Behavior checks for the footprint experiment and measurement harness."""

import importlib.util
import os
import subprocess
import tempfile
import unittest
from http.client import HTTPConnection
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location(
    "measure", Path(__file__).with_name("measure.py")
)
measure = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(measure)
BINARY = Path(
    os.environ.get("LIMEOS_SPIKE_TEST_BINARY", ROOT / "target/release/limeos-footprint")
)


class ProbeTests(unittest.TestCase):
    def test_real_http_and_sqlite_path_has_no_mutation_route(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with (
                measure.fixture_socket(root) as socket,
                measure.start_probe(BINARY, root / "state", socket) as probe,
            ):
                process, ready, _elapsed = probe
                overview = measure.request(ready["listen"], "/api/v1/overview")
                self.assertEqual(overview["containers"]["running"], 18)
                self.assertEqual(overview["containers"]["total"], 19)
                self.assertEqual(overview["sqlite_rows"], 1)
                before = measure.read_counters(process.pid)
                for _ in range(20):
                    measure.request(ready["listen"], "/api/v1/overview")
                after = measure.read_counters(process.pid)
                self.assertEqual(before["write_bytes"], after["write_bytes"])
                connection = HTTPConnection(*ready["listen"].rsplit(":", 1), timeout=5)
                try:
                    connection.request("POST", "/api/v1/overview")
                    self.assertEqual(connection.getresponse().status, 405)
                finally:
                    connection.close()

    def test_docker_failure_does_not_produce_ready_or_empty_success(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with (
                self.assertRaises(RuntimeError),
                measure.start_probe(BINARY, root / "state", str(root / "absent.sock")),
            ):
                self.fail(
                    "unavailable Docker must not measure a successful empty server"
                )

    def test_public_listener_is_rejected_before_docker_or_database_access(self):
        with tempfile.TemporaryDirectory() as directory:
            result = subprocess.run(
                [str(BINARY)],
                capture_output=True,
                text=True,
                timeout=5,
                check=False,
                env={
                    **os.environ,
                    "LIMEOS_SPIKE_STATE_DIR": directory,
                    "LIMEOS_SPIKE_LISTEN": "0.0.0.0:8999",
                },
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("must bind to loopback", result.stderr)
            self.assertEqual(list(Path(directory).iterdir()), [])


if __name__ == "__main__":
    unittest.main()
