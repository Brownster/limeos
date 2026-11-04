"""Native fixture-boundary checks; no installed runtime acceptance."""

import copy
import gc
import importlib.util
import io
import json
import os
import sqlite3
import struct
import subprocess
import sys
import tempfile
import unittest
from contextlib import closing, redirect_stdout
from pathlib import Path
from unittest import mock

SOURCE = Path(__file__).resolve().parents[1] / "privileged_vm/p04_automation_guest.py"
SPEC = importlib.util.spec_from_file_location("b1_installed_fixture", SOURCE)
guest = importlib.util.module_from_spec(SPEC)
sys.path.insert(0, str(SOURCE.parent))
try:
    SPEC.loader.exec_module(guest)
finally:
    sys.path.pop(0)


class AutomationFixtureGuards(unittest.TestCase):
    def probe(self, data, *, peer_uid=0, client_uid=1000, times=None):
        class Stream:
            def __init__(self):
                self.remaining = data
                self.sent = []

            def __enter__(self):
                return self

            def __exit__(self, *_):
                pass

            def connect(self, path):
                if path != "/run/limeos-storage-reader/executor.sock":
                    raise AssertionError("unexpected probe socket")

            def settimeout(self, value):
                if not 0 < value <= 4:
                    raise AssertionError("probe renewed its original deadline")

            def getsockopt(self, *_):
                return struct.pack("3i", 12, peer_uid, 0)

            def sendall(self, value):
                self.sent.append(value)

            def recv(self, size):
                # Every header and body is fragmented into one-byte reads.
                value, self.remaining = self.remaining[:1], self.remaining[1:]
                if size < 1:
                    raise AssertionError("unbounded or empty read")
                return value

        stream = Stream()
        output = io.StringIO()
        with (
            mock.patch("socket.socket", return_value=stream),
            mock.patch("os.geteuid", return_value=client_uid),
            mock.patch("pwd.getpwnam", return_value=mock.Mock(pw_uid=1000)),
            mock.patch("sys.argv", ["probe", "a" * 64]),
            mock.patch("time.monotonic", side_effect=times, return_value=0),
            redirect_stdout(output),
        ):
            # The exact fixed guest observer runs only under private mocked I/O.
            exec(guest.HEALTH_PROBE, {})  # noqa: S102
        return json.loads(output.getvalue()), stream

    def test_health_probe_bounds_fragmentation_peer_and_early_eof(self):
        reply = {"version": 1, "ready": False, "error": "unavailable"}
        body = json.dumps(reply).encode()
        observed, stream = self.probe(struct.pack("!I", len(body)) + body)
        self.assertEqual(observed["reply"], reply)
        self.assertEqual(observed["client_uid"], 1000)
        request = json.loads(stream.sent[0][4:])
        self.assertEqual(
            request, {"operation": "container_health", "version": 1, "nonce": "a" * 64}
        )
        for data in (
            b"",
            b"\0\0",
            struct.pack("!I", 65537),
            struct.pack("!I", 0),
            struct.pack("!I", 9) + b"{}",
        ):
            with self.subTest(data=data), self.assertRaises((ValueError, EOFError)):
                self.probe(data)
        with self.assertRaises(PermissionError):
            self.probe(struct.pack("!I", len(body)) + body, peer_uid=1000)
        with self.assertRaises(PermissionError):
            self.probe(struct.pack("!I", len(body)) + body, client_uid=0)

    def test_health_probe_keeps_one_absolute_deadline(self):
        with self.assertRaises(TimeoutError):
            self.probe(struct.pack("!I", 2) + b"{}", times=[0, 0, 0, 1, 2, 3, 4.1])

    def test_failure_observation_is_read_only_and_isolates_missing_sections(self):
        with (
            mock.patch.object(guest, "disposable_guard"),
            mock.patch.object(
                guest, "request", return_value={"items": [], "partial": True}
            ) as request,
            mock.patch.object(
                guest, "health_probe", side_effect=subprocess.TimeoutExpired("probe", 5)
            ) as probe,
            mock.patch.object(
                guest, "service_snapshot", side_effect=[{"pid": 11}, OSError("gone")]
            ),
        ):
            value = guest.failure_snapshot()
        request.assert_called_once_with("/schedules")
        probe.assert_called_once_with()
        self.assertTrue(value["runtime"]["available"])
        self.assertEqual(value["health"]["error_type"], "TimeoutExpired")
        self.assertTrue(value["core"]["available"])
        self.assertFalse(value["reader"]["available"])
        self.assertIn("not the timer", value["scope"])

    def test_failure_observation_bounds_each_section_and_preserves_separate_output(
        self,
    ):
        with (
            mock.patch.object(guest, "disposable_guard"),
            mock.patch.object(
                guest, "request", return_value={"oversized": "x" * 65536}
            ),
            mock.patch.object(
                guest, "health_probe", return_value={"reply": {"ready": False}}
            ),
            mock.patch.object(guest, "service_snapshot", return_value={}),
        ):
            value = guest.failure_snapshot()
        self.assertFalse(value["runtime"]["available"])
        self.assertTrue(value["health"]["available"])
        evidence = {"error": "original failure", "large": "x" * 200000}
        stdout = io.StringIO()
        with (
            mock.patch.object(guest, "failure_snapshot", return_value=value),
            redirect_stdout(stdout),
        ):
            guest.record_failure_observation(evidence)
        standalone = stdout.getvalue().removeprefix("B1_OBSERVATION ")
        self.assertLessEqual(len(standalone.encode()), 96 * 1024)
        self.assertEqual(json.loads(standalone)["health"], value["health"])
        self.assertEqual(evidence["error"], "original failure")
        self.assertEqual(
            json.loads(guest.failure_packet(evidence))["error"], "original failure"
        )

    def test_wrong_uid_or_hostname_refuses_before_any_effect(self):
        for uid, hostname in ((1000, "limeos-p01-test"), (0, "production")):
            with (
                self.subTest(uid=uid, hostname=hostname),
                mock.patch.object(guest.os, "geteuid", return_value=uid),
                mock.patch.object(guest.Path, "read_text", return_value=hostname),
                mock.patch.object(guest, "run") as command,
                mock.patch.object(guest, "history") as history,
                self.assertRaises(SystemExit),
            ):
                guest.main()
            command.assert_not_called()
            history.assert_not_called()

    def test_oversized_failure_retains_explicit_unavailable_details(self):
        value = guest.failure_packet(
            {
                "stage": "incident",
                "summary": "fail",
                "error": "refused",
                "rows": "x" * 200_000,
            }
        )
        self.assertLessEqual(len(value.encode()), 128 * 1024)
        parsed = json.loads(value)
        self.assertEqual(parsed["stage"], "incident")
        self.assertIn("unavailable", parsed["details"])

    def test_read_only_history_closes_sqlite_on_success_and_failure_without_gc(self):
        with tempfile.TemporaryDirectory(dir=SOURCE.parents[2] / ".cache") as directory:
            path = Path(directory) / "history.sqlite"
            with closing(sqlite3.connect(path)) as conn:
                conn.executescript("""
                    CREATE TABLE automation_schedules(id,revision,paused,settled_through);
                    CREATE TABLE automation_occurrences(schedule,revision,due,job);
                    CREATE TABLE jobs(id,state,digest,intent);
                    CREATE TABLE automation_observation_results(job,outcome,reason,sequence,complete);
                    CREATE TABLE automation_alerts(owner,key,body);
                    CREATE TABLE automation_notifications(id,job,body);
                """)
            before = len(os.listdir("/proc/self/fd"))
            enabled = gc.isenabled()
            gc.disable()
            try:
                for _ in range(40):
                    self.assertEqual(
                        guest.history(path),
                        {
                            key: []
                            for key in (
                                "schedules",
                                "occurrences",
                                "jobs",
                                "results",
                                "alerts",
                                "notifications",
                            )
                        },
                    )
                self.assertEqual(len(os.listdir("/proc/self/fd")), before)
                with closing(sqlite3.connect(path)) as conn:
                    conn.execute("DROP TABLE automation_alerts")
                for _ in range(40):
                    with self.assertRaises(sqlite3.OperationalError):
                        guest.history(path)
                self.assertEqual(len(os.listdir("/proc/self/fd")), before)
            finally:
                if enabled:
                    gc.enable()

    def test_duplicate_jobs_wrong_bindings_and_complete_results_are_refused(self):
        valid = {
            "occurrences": [["schedule", 1, 200, "job"]],
            "jobs": [["job", "succeeded", "digest"]],
            "results": [["job", "succeeded", "observed", 1, 0]],
            "notifications": [["notice", "job", json.dumps({"channel": "in_app"})]],
        }
        guest.assert_identities(valid)
        for part in ("occurrences", "jobs", "results"):
            invalid = copy.deepcopy(valid)
            invalid[part].append(invalid[part][0])
            with self.subTest(part=part), self.assertRaises(ValueError):
                guest.assert_identities(invalid)
        for part, row, index, value in (
            ("occurrences", 0, 3, "other"),
            ("results", 0, 4, 1),
            ("results", 0, 1, "failed"),
            ("notifications", 0, 1, "other"),
        ):
            invalid = copy.deepcopy(valid)
            invalid[part][row][index] = value
            with self.subTest(part=part, value=value), self.assertRaises(ValueError):
                guest.assert_identities(invalid)

    def test_notification_requires_delivery_exact_phase_and_original_generation(self):
        value = {
            "event": {"key": "container:full-id", "event": "recovery", "generation": 2},
            "delivery": {"state": "pending"},
        }
        snapshot = {"notifications": [["notice", "job", json.dumps(value)]]}
        self.assertIsNone(
            guest.notification(snapshot, "container:full-id", "recovery", 2)
        )
        value["delivery"]["state"] = "delivered"
        snapshot["notifications"][0][2] = json.dumps(value)
        self.assertEqual(
            guest.notification(snapshot, "container:full-id", "recovery", 2), value
        )
        self.assertIsNone(
            guest.notification(snapshot, "container:other-id", "recovery", 2)
        )
        with self.assertRaisesRegex(ValueError, "generation changed"):
            guest.notification(snapshot, "container:full-id", "recovery", 1)
        snapshot["notifications"].append(snapshot["notifications"][0])
        with self.assertRaisesRegex(ValueError, "duplicate notification"):
            guest.notification(snapshot, "container:full-id", "recovery", 2)


if __name__ == "__main__":
    unittest.main()
