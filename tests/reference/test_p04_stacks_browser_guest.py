"""Fixture boundary regressions; these do not qualify an installed Stack."""

import copy
import importlib.util
import io
import json
import sqlite3
import sys
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path
from unittest import mock

SOURCE = (
    Path(__file__).resolve().parents[1] / "privileged_vm/p04_stacks_browser_guest.py"
)
SPEC = importlib.util.spec_from_file_location("b2_browser_fixture", SOURCE)
guest = importlib.util.module_from_spec(SPEC)
sys.path.insert(0, str(SOURCE.parent))
try:
    SPEC.loader.exec_module(guest)
finally:
    sys.path.pop(0)


def empty_report(declared):
    return {
        "catalog": {"state": "fresh"},
        "engine": {"state": "fresh"},
        "engine_matches_catalog": True,
        "standalone_containers": 0,
        "other_projects": [],
        "stacks": [
            {
                "stack": row["stack"],
                "project": row["project"],
                "working_directory": row["working_directory"],
                "config_files": [row["working_directory"] + "/compose.yaml"],
                "files": {"state": "not_inspected"},
                "runtime": "no_containers",
                "issues": [],
                "counts": {
                    "ordinary": 0,
                    "running": 0,
                    "one_offs": 0,
                    "one_off_running": 0,
                },
                "missing_services": len(row["services"]),
                "services": [
                    {
                        "name": name,
                        "declared": True,
                        "counts": {
                            "ordinary": 0,
                            "running": 0,
                            "one_offs": 0,
                            "one_off_running": 0,
                        },
                        "members": [],
                    }
                    for name in row["services"]
                ],
            }
            for row in declared["stacks"]
        ],
    }


class StackFixtureBoundaries(unittest.TestCase):
    def test_authority_observer_uses_real_migrated_schema_without_writes(self):
        schema = SOURCE.parents[2] / "crates/persistence/src"
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "core.sqlite"
            with sqlite3.connect(path) as conn:
                conn.executescript((schema / "schema.sql").read_text())
                for version in range(2, 15):
                    conn.executescript(
                        (schema / f"migration-v{version}.sql").read_text()
                    )
            before = path.read_bytes()
            value = guest.authority_counts(path)
            self.assertTrue(all(count == 0 for count in value.values()))
            self.assertIn("container_plans", value)
            self.assertIn("compose_plans", value)
            self.assertEqual(path.read_bytes(), before)

    def declaration(self, maximum=False):
        with mock.patch.object(guest.fixture, "sha", return_value="a" * 64):
            return guest.catalog("genuine-fixture-engine", maximum=maximum)

    def test_production_and_nonroot_refuse_before_effects(self):
        for uid, hostname in ((1000, "limeos-p01-test"), (0, "hollys-pi")):
            with (
                mock.patch.object(guest.fixture.os, "geteuid", return_value=uid),
                mock.patch.object(guest.Path, "read_text", return_value=hostname),
                mock.patch.object(guest, "run") as run,
                self.assertRaises(SystemExit),
            ):
                guest.main()
            run.assert_not_called()

    def test_complete_catalog_preserves_paths_and_maximum(self):
        declared = self.declaration(maximum=True)
        self.assertEqual(len(declared["stacks"]), 8)
        self.assertTrue(all(len(row["services"]) == 16 for row in declared["stacks"]))
        self.assertEqual(
            declared["stacks"][7]["working_directory"], "/opt/limeos-b2-fixture/Stack7"
        )
        self.assertEqual(len(guest.LAYOUT), 16)
        self.assertEqual(sum(row[3] for row in guest.LAYOUT), 8)

    def test_checker_refuses_truncated_or_false_current_views(self):
        declared = self.declaration()
        original = empty_report(declared)
        guest.check_report(original, declared, [])
        variants = []
        value = copy.deepcopy(original)
        value["stacks"].pop()
        variants.append(value)
        value = copy.deepcopy(original)
        value["catalog"]["state"] = "stale"
        variants.append(value)
        value = copy.deepcopy(original)
        value["stacks"][0]["working_directory"] = value["stacks"][0][
            "working_directory"
        ].lower()
        variants.append(value)
        value = copy.deepcopy(original)
        value["stacks"][0]["files"] = {"state": "inspected"}
        variants.append(value)
        value = copy.deepcopy(original)
        value["stacks"][0]["runtime"] = "running"
        variants.append(value)
        value = copy.deepcopy(original)
        value["standalone_containers"] = 1
        variants.append(value)
        for value in variants:
            with self.subTest(value=value), self.assertRaises(ValueError):
                guest.check_report(value, declared, [])

    def test_checker_binds_oneoff_and_full_ids_independently_of_totals(self):
        declared = self.declaration()
        value = empty_report(declared)
        member = {
            "id": "1" * 64,
            "stack": "fixture2",
            "service": "web",
            "one_off": True,
            "running": True,
        }
        row = value["stacks"][2]
        row["counts"] = {
            "ordinary": 0,
            "running": 0,
            "one_offs": 1,
            "one_off_running": 1,
        }
        row["services"][0]["counts"] = row["counts"].copy()
        row["services"][0]["members"] = [
            {
                "container": {"resource": "container:" + member["id"], "running": True},
                "one_off": True,
            }
        ]
        guest.check_report(value, declared, [member])
        for field, replacement in (
            ("resource", "container:" + "2" * 64),
            ("running", False),
        ):
            changed = copy.deepcopy(value)
            changed["stacks"][2]["services"][0]["members"][0]["container"][field] = (
                replacement
            )
            with self.assertRaises(ValueError):
                guest.check_report(changed, declared, [member])

    def test_failure_packet_excludes_exception_secrets(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "result.json"
            stdout = io.StringIO()
            with (
                mock.patch.object(guest.fixture, "disposable_guard"),
                mock.patch.object(
                    guest.browser, "oom_events", return_value={"oom_kill": 0}
                ),
                mock.patch.object(
                    guest, "run", side_effect=RuntimeError("private cookie/password")
                ),
                mock.patch.object(sys, "argv", [str(SOURCE), directory, str(output)]),
                redirect_stdout(stdout),
                self.assertRaises(RuntimeError),
            ):
                guest.main()
            self.assertEqual(json.loads(output.read_text())["summary"], "fail")
            self.assertNotIn("private cookie", output.read_text() + stdout.getvalue())

    def test_observer_refuses_errors_or_missing_worker_roles(self):
        observer = guest.ReaderResources({})
        observer.value["samples"] = 1
        with self.assertRaises(ValueError):
            observer.finish()
        observer.value["processes"] = {
            str(n): {"role": role}
            for n, role in enumerate(("manager", "guardian", "worker"))
        }
        self.assertIs(observer.finish(), observer.value)
        observer.value["errors"] = ["PermissionError"]
        with self.assertRaises(ValueError):
            observer.finish()


if __name__ == "__main__":
    unittest.main()
