"""Setup refusal and timeout cases; no real service, RPC or guest is opened."""

import importlib.util
import json
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest import mock

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location(
    "fixture_readiness", ROOT / "tests/privileged_vm/p04_fixture_readiness.py"
)
fixture = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(fixture)


class FixtureReadinessTests(unittest.TestCase):
    def setUp(self):
        self.clock = 0.0
        for patch in (
            mock.patch.object(fixture.os, "geteuid", return_value=0),
            mock.patch.object(
                fixture.socket, "gethostname", return_value="limeos-p01-test"
            ),
            mock.patch.object(
                fixture.time, "monotonic", side_effect=lambda: self.clock
            ),
            mock.patch.object(fixture.time, "sleep", side_effect=self.advance),
        ):
            patch.start()
            self.addCleanup(patch.stop)

    def advance(self, seconds):
        self.clock += seconds

    def reader(self, values):
        run = mock.Mock(
            side_effect=[SimpleNamespace(stdout=json.dumps(v)) for v in values]
        )
        with mock.patch.object(fixture.Path, "is_socket", return_value=True):
            return fixture.wait_storage_reader(run, "fixed-read-client"), run

    def test_reader_busy_then_real_response_uses_only_fixed_read_request(self):
        report, run = self.reader(
            [
                {"version": 1, "ready": False, "error": "overloaded"},
                {"version": 1, "ready": True, "storage": {"host_root_mount_id": 27}},
            ]
        )
        self.assertEqual([v["ready"] for v in report["observations"]], [False, True])
        for call in run.call_args_list:
            self.assertEqual(
                json.loads(call.args[-1]),
                {"operation": "storage_inventory", "version": 1},
            )
            self.assertIn("limeos-core", call.args)
            self.assertLessEqual(call.kwargs["timeout"], 6)

    def test_reader_never_turns_permanent_or_malformed_refusal_into_readiness(self):
        for value in (
            {"version": 1, "ready": False, "error": "forbidden"},
            {"version": 1, "ready": False, "error": "unavailable"},
            {"version": 2, "ready": True, "storage": {}},
            {"version": True, "ready": True, "storage": {}},
            {"version": 1, "ready": True, "storage": []},
            {"version": 1, "ready": False, "error": "overloaded", "storage": {}},
            [],
        ):
            error = ValueError if isinstance(value, dict) else TypeError
            with self.subTest(value=value), self.assertRaises(error):
                self.reader([value])

    def test_reader_socket_and_busy_deadlines_cannot_report_success(self):
        with mock.patch.object(fixture.Path, "is_socket", return_value=False):
            run = mock.Mock()
            with self.assertRaises(TimeoutError):
                fixture.wait_storage_reader(run, "fixed-read-client")
            run.assert_not_called()
        run = mock.Mock(
            return_value=SimpleNamespace(
                stdout=json.dumps({"version": 1, "ready": False, "error": "overloaded"})
            )
        )
        with (
            mock.patch.object(fixture.Path, "is_socket", return_value=True),
            self.assertRaises(TimeoutError),
        ):
            fixture.wait_storage_reader(run, "fixed-read-client")
        self.assertEqual(run.call_count, 64)

    def test_unknown_host_refuses_before_any_command(self):
        with mock.patch.object(fixture.os, "geteuid", return_value=1000):
            for function, args in (
                (fixture.wait_storage_reader, ("probe",)),
                (fixture.wait_udev_workers, ()),
            ):
                run = mock.Mock()
                with self.assertRaises(ValueError):
                    function(run, *args)
                run.assert_not_called()

    def udev(self, workers, changed=False):
        run = mock.Mock(return_value=SimpleNamespace(stdout="278\n"))

        def read(path):
            if path.name == "children":
                return workers(self.clock)
            ticks = 101 if changed and self.clock else 100
            return "278 (systemd-udevd) " + " ".join(["0"] * 19 + [str(ticks)])

        with mock.patch.object(fixture, "_read", side_effect=read):
            return fixture.wait_udev_workers(run), run

    def test_udev_waits_for_natural_exit_without_stopping_or_modifying_service(self):
        report, run = self.udev(lambda now: "7137 7138" if now < 0.2 else "")
        self.assertEqual(report["initial_children"], [7137, 7138])
        self.assertEqual(report["final_children"], [])
        self.assertEqual(
            [call.args[:2] for call in run.call_args_list],
            [("udevadm", "settle"), ("systemctl", "show")],
        )
        self.assertFalse(report["service_stopped_or_modified"])

    def test_udev_parent_change_persistent_workers_and_invalid_membership_refuse(self):
        for workers, changed, error in (
            (lambda _: "7137", True, ValueError),
            (lambda _: "7137", False, TimeoutError),
            (lambda _: "7137/../evil", False, ValueError),
            (lambda _: "1 " * 65, False, ValueError),
        ):
            self.clock = 0
            with self.subTest(changed=changed, error=error), self.assertRaises(error):
                self.udev(workers, changed=changed)

    def test_workers_exiting_at_the_deadline_cannot_report_quiescence(self):
        with (
            mock.patch.object(
                fixture.time, "sleep", side_effect=lambda _: self.advance(35)
            ),
            self.assertRaises(TimeoutError),
        ):
            self.udev(lambda now: "7137" if now < 35 else "")

    def test_proc_read_bound_is_enforced_before_parsing(self):
        (ROOT / ".cache").mkdir(exist_ok=True)
        with tempfile.TemporaryDirectory(dir=ROOT / ".cache") as directory:
            path = Path(directory) / "stat"
            path.write_bytes(b"x" * 4097)
            with self.assertRaises(ValueError):
                fixture._read(path)


if __name__ == "__main__":
    unittest.main()
