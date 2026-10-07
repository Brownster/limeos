"""Crash fixtures must wait for the executor listener, not a stale unit state."""

import importlib.util
import io
import json
import socket
import struct
import subprocess
import sys
import unittest
from contextlib import redirect_stdout
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import Mock, patch

SOURCE = Path(__file__).resolve().parents[1] / "privileged_vm/p03_guest.py"
SPEC = importlib.util.spec_from_file_location("crash_readiness_fixture", SOURCE)
FIXTURE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(FIXTURE)


class CrashReadinessTests(unittest.TestCase):
    def probe_code(self):
        probe = []

        def backend(*argv, **_):
            probe.append(argv[-1])
            return SimpleNamespace(returncode=0, stdout='{"ready":true}', stderr="")

        with patch.object(FIXTURE, "run", side_effect=backend):
            FIXTURE.wait_container_ready()
        return probe[0]

    def test_active_unit_and_terminal_job_do_not_bypass_unready_executor(self):
        observations = iter((False, False, True))
        health_probes = []
        job_state = "needs_intervention"

        def backend(*argv, **_):
            if argv[0] == "systemctl":
                return SimpleNamespace(returncode=0, stdout="active\n", stderr="")
            ready = next(observations)
            health_probes.append(ready)
            return SimpleNamespace(
                returncode=0, stdout=json.dumps({"ready": ready}), stderr=""
            )

        with (
            patch.object(FIXTURE, "run", side_effect=backend),
            patch.object(FIXTURE.time, "sleep"),
            patch.object(FIXTURE.time, "monotonic", return_value=0),
        ):
            FIXTURE.wait_recovered_service("limeos-containerd")
        self.assertEqual(job_state, "needs_intervention")
        self.assertEqual(health_probes, [False, False, True])

    def test_missing_executor_restart_fails_at_recovery_deadline(self):
        def backend(*argv, **_):
            if argv[0] == "systemctl":
                return SimpleNamespace(returncode=0, stdout="active\n", stderr="")
            return SimpleNamespace(
                returncode=1, stdout="", stderr="fixture socket not ready"
            )

        with (
            patch.object(FIXTURE, "run", side_effect=backend),
            patch.object(FIXTURE.time, "sleep"),
            patch.object(FIXTURE.time, "monotonic", side_effect=(0, 0, 44.9, 45.1)),
            self.assertRaisesRegex(AssertionError, "fixture socket not ready"),
        ):
            FIXTURE.wait_recovered_service("limeos-containerd")

    def test_core_recovery_preserves_existing_active_state_wait(self):
        states = iter(("activating\n", "active\n"))
        calls = []

        def backend(*argv, **_):
            calls.append(argv)
            return SimpleNamespace(returncode=0, stdout=next(states), stderr="")

        with (
            patch.object(FIXTURE, "run", side_effect=backend),
            patch.object(FIXTURE.time, "sleep"),
            patch.object(FIXTURE.time, "monotonic", return_value=0),
        ):
            FIXTURE.wait_recovered_service("limeos-core")
        self.assertEqual(len(calls), 2)
        self.assertTrue(all(call[0] == "systemctl" for call in calls))

    def test_truncated_probe_body_fails_without_repeating_eof(self):
        code = self.probe_code()

        class TruncatedSocket:
            def __init__(self):
                self.reads = 0

            def settimeout(self, _):
                pass

            def connect(self, _):
                pass

            def sendall(self, _):
                pass

            def recv(self, _):
                self.reads += 1
                if self.reads == 1:
                    return struct.pack("!I", 10)
                if self.reads == 2:
                    return b""
                raise AssertionError("probe spun after EOF")

        connection = TruncatedSocket()
        with (
            patch.object(socket, "socket", return_value=connection),
            self.assertRaisesRegex(OSError, "closed"),
        ):
            exec(code, {})  # noqa: S102 - exercise the repository's fixed IPC probe
        self.assertEqual(connection.reads, 2)

    def test_fragmented_health_header_and_body_are_read_exactly(self):
        code = self.probe_code()
        payload = b'{"ready":true}'
        header = struct.pack("!I", len(payload))
        connection = Mock()
        connection.recv.side_effect = [header[:1], header[1:], payload[:2], payload[2:]]
        output = io.StringIO()
        with (
            patch.object(socket, "socket", return_value=connection),
            redirect_stdout(output),
        ):
            exec(code, {})  # noqa: S102 - exercise the repository's fixed IPC probe
        self.assertEqual(json.loads(output.getvalue()), {"ready": True})
        self.assertEqual(connection.recv.call_count, 4)

    def test_invalid_health_frame_size_is_refused_before_body_read(self):
        code = self.probe_code()
        for size in (0, 65537, 2**31):
            connection = Mock()
            connection.recv.return_value = struct.pack("!I", size)
            with (
                self.subTest(size=size),
                patch.object(socket, "socket", return_value=connection),
                self.assertRaisesRegex(OSError, "frame size"),
            ):
                exec(code, {})  # noqa: S102 - exercise the repository's fixed IPC probe
            self.assertEqual(connection.recv.call_count, 1)

    def test_hanging_probe_child_is_killed_at_remaining_deadline(self):
        original_run = subprocess.run
        timeouts = []

        def child(_args, **kwargs):
            timeout = kwargs.get("timeout")
            self.assertIsNotNone(timeout, "health subprocess has no deadline")
            self.assertGreater(timeout, 0)
            self.assertLessEqual(timeout, 0.05)
            timeouts.append(timeout)
            return original_run(
                [sys.executable, "-c", "import time; time.sleep(30)"], **kwargs
            )

        with (
            patch.object(FIXTURE.subprocess, "run", side_effect=child),
            self.assertRaisesRegex(AssertionError, "timed out"),
        ):
            FIXTURE.wait_container_ready(timeout=0.05)
        self.assertTrue(timeouts)
