"""Pure source-fixture guards; these do not qualify a mount or run a guest."""

import contextlib
import importlib.util
import io
import json
import os
import stat
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


if __name__ == "__main__":
    unittest.main()
