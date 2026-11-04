"""Native fixture-boundary checks; no installed runtime acceptance."""

import copy
import gc
import importlib.util
import json
import os
import sqlite3
import sys
import tempfile
import unittest
from contextlib import closing
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
