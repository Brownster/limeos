"""Pure source-fixture guards; these do not qualify a mount or run a guest."""

import contextlib
import gc
import importlib.util
import io
import json
import os
import sqlite3
import stat
import tempfile
import unittest
from pathlib import Path
from unittest import mock

SOURCE = Path(__file__).resolve().parents[1] / "privileged_vm/p04_mounts_guest.py"
SPEC = importlib.util.spec_from_file_location("a1_mount_guest_fixture", SOURCE)
guest = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(guest)


class MountFixtureGuards(unittest.TestCase):
    def test_non_guest_refusal_prints_failure_without_commands_or_publication(self):
        output = io.StringIO()
        with (
            mock.patch.object(guest.os, "geteuid", return_value=1000),
            mock.patch.object(guest, "run") as command,
            mock.patch.object(guest, "publish") as publish,
            contextlib.redirect_stdout(output),
            self.assertRaises(RuntimeError),
        ):
            guest.qualify("/opt/limeos-repo", "/root/result.json", "0.4.4", 10)
        command.assert_not_called()
        publish.assert_not_called()
        record = json.loads(output.getvalue().removeprefix("A1_FAILURE "))
        self.assertEqual(record["summary"], "fail")
        self.assertIsNone(record["source_commit"])
        self.assertIn("disposable guest", record["error"])

    def test_failure_stdout_discards_oversized_details_without_claiming_persistence(
        self,
    ):
        record = guest.failure_packet(
            {
                "summary": "fail",
                "stage": "guard",
                "error": "refused",
                "source_commit": None,
                "details": "x" * (guest.LIMIT + 1),
            }
        )
        self.assertLessEqual(len(record.encode()), guest.LIMIT)
        self.assertIn("not assumed", json.loads(record)["details"])

    def test_independent_mountinfo_parser_keeps_identity_and_refuses_ambiguity(self):
        row = "20 1 253:65 / /mnt/Data\\040Set rw,nosuid,nodev - ext4 /dev/vde1 rw\n"
        parsed = guest.parse_mounts(row)[0]
        self.assertEqual(parsed["device"], {"major": 253, "minor": 65})
        self.assertEqual(parsed["target"], "/mnt/Data Set")
        self.assertEqual(set(parsed["options"]), {"rw", "nosuid", "nodev"})
        for invalid in (
            row + row,
            row.replace("\\040", "\\041"),
            row.replace(" - ", " - - "),
        ):
            with self.assertRaises(ValueError):
                guest.parse_mounts(invalid)
        source = mock.Mock()
        source.stat.return_value.st_mode = stat.S_IFBLK
        source.stat.return_value.st_rdev = os.makedev(253, 65)
        source.is_symlink.return_value = False
        target = dict(parsed, target=str(guest.TARGET))
        root = dict(parsed, target="/", mount_id=1)
        facts = {"source_device": parsed["device"], "host_root_mount_id": 1}
        for roots in ([], [root, dict(root, mount_id=2)]):
            with (
                self.subTest(root_count=len(roots)),
                mock.patch.object(guest, "SOURCE", source),
                mock.patch.object(guest, "mounts", return_value=[target, *roots]),
                mock.patch.object(guest, "run", return_value=mock.Mock(stdout="uuid")),
                mock.patch.object(guest, "directory") as destination,
                self.assertRaisesRegex(ValueError, "exactly one host root mount"),
            ):
                guest.mounted_snapshot(facts, "uuid")
            destination.assert_not_called()


class FailureObservations(unittest.TestCase):
    def setUp(self):
        self.action = "a" * 64
        self.directory = tempfile.TemporaryDirectory(dir=SOURCE.parents[2] / ".cache")
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name) / "root.sqlite"
        self.core = Path(self.directory.name) / "core.sqlite"
        with contextlib.closing(sqlite3.connect(self.root)) as database, database:
            database.executescript(
                "CREATE TABLE receipts(action PRIMARY KEY,digest,state,body);"
                "CREATE TABLE locks(resource,action);"
            )
        with contextlib.closing(sqlite3.connect(self.core)) as database, database:
            database.executescript(
                "CREATE TABLE resource_locks(resource,job);"
                "CREATE TABLE job_resources(resource,job);"
                "INSERT INTO resource_locks VALUES('foreign','other');"
            )
        for patch in (
            mock.patch.object(guest, "ROOT_DB", self.root),
            mock.patch.object(guest, "CORE_DB", self.core),
            mock.patch.object(guest.os, "geteuid", return_value=0),
            mock.patch.object(
                guest.socket, "gethostname", return_value="limeos-p01-test"
            ),
            mock.patch.object(guest, "mounts", return_value=[]),
        ):
            patch.start()
            self.addCleanup(patch.stop)

    def store(self, body):
        with contextlib.closing(sqlite3.connect(self.root)) as database, database:
            database.execute(
                "INSERT INTO receipts VALUES(?,?,?,?)",
                (self.action, "b" * 64, "outcome_unknown", body),
            )

    def test_real_sqlite_observations_preserve_receipt_and_claims_without_writes(self):
        receipt = {
            "plan": {"action": self.action},
            "state": "outcome_unknown",
            "quiescence": {"state": "pending", "helper": None},
            "after": None,
        }
        body = json.dumps(receipt)
        self.store(body)
        with contextlib.closing(sqlite3.connect(self.root)) as database, database:
            database.execute("INSERT INTO locks VALUES('device:1:2',?)", (self.action,))
        with contextlib.closing(sqlite3.connect(self.core)) as database, database:
            for table in ("resource_locks", "job_resources"):
                database.execute(
                    f"INSERT INTO {table} VALUES('device:1:2',?)", (self.action,)
                )
        before = (self.root.read_bytes(), self.core.read_bytes())
        with mock.patch.object(
            guest,
            "mounts",
            return_value=[
                {"target": str(guest.TARGET), "mount_id": 44},
                {"target": "/other"},
            ],
        ):
            report = guest.execution_failure_snapshot(self.action)
        self.assertEqual(report["root_receipt"]["receipt"], receipt)
        self.assertEqual(report["root_receipt"]["stored_state"], "outcome_unknown")
        self.assertEqual(report["root_receipt"]["body_sha256"], guest.text_digest(body))
        for name in ("root_claims", "core_claims", "core_required_resources"):
            self.assertEqual(report[name], ["device:1:2"])
        self.assertEqual(
            report["target_mount_rows"], [{"target": str(guest.TARGET), "mount_id": 44}]
        )
        self.assertEqual(before, (self.root.read_bytes(), self.core.read_bytes()))
        self.assertIn("not an atomic snapshot or authority", report["scope"])

    def test_missing_receipt_is_observed_without_creating_one(self):
        report = guest.execution_failure_snapshot(self.action)
        self.assertEqual(report["root_receipt"], {"present": False})
        self.assertEqual(report["root_claims"], [])

    def test_read_connections_close_without_waiting_for_garbage_collection(self):
        was_enabled = gc.isenabled()
        gc.disable()
        try:
            before = set(os.listdir("/proc/self/fd"))
            for _ in range(32):
                self.assertEqual(
                    guest.query(self.root, "SELECT count(*) FROM receipts"), [(0,)]
                )
            with self.assertRaises(sqlite3.OperationalError):
                guest.query(self.root, "SELECT missing FROM no_such_table")
            self.assertEqual(before, set(os.listdir("/proc/self/fd")))
        finally:
            if was_enabled:
                gc.enable()
            gc.collect()

    def test_corrupt_or_oversized_receipt_does_not_hide_other_observations(self):
        for body in ("not-json", "é" * 32769, '{"plan":{"action":"other"}}'):
            with self.subTest(size=len(body.encode())):
                with (
                    contextlib.closing(sqlite3.connect(self.root)) as database,
                    database,
                ):
                    database.execute("DELETE FROM receipts")
                self.store(body)
                report = guest.execution_failure_snapshot(self.action)
                self.assertFalse(report["root_receipt"]["available"])
                self.assertEqual(report["core_claims"], [])
                self.assertEqual(report["target_mount_rows"], [])

    def test_excess_count_or_byte_length_cannot_report_complete_claims(self):
        for resources in ([str(i) for i in range(33)], ["é" * 129]):
            with self.subTest(count=len(resources)):
                with (
                    contextlib.closing(sqlite3.connect(self.root)) as database,
                    database,
                ):
                    database.execute("DELETE FROM locks")
                    database.executemany(
                        "INSERT INTO locks VALUES(?,?)",
                        [(r, self.action) for r in resources],
                    )
                report = guest.execution_failure_snapshot(self.action)
                self.assertFalse(report["root_claims"]["available"])
                self.assertEqual(report["core_required_resources"], [])

    def test_invalid_host_or_action_cannot_open_a_database(self):
        with mock.patch.object(guest, "query") as read:
            with (
                mock.patch.object(
                    guest.socket, "gethostname", return_value="production"
                ),
                self.assertRaises(ValueError),
            ):
                guest.execution_failure_snapshot(self.action)
            for action in (None, "../other", "A" * 64):
                with self.assertRaises(ValueError):
                    guest.execution_failure_snapshot(action)
        read.assert_not_called()


if __name__ == "__main__":
    unittest.main()
