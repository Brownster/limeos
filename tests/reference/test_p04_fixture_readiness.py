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
            with (
                mock.patch.object(fixture, "_docker_json") as read,
                self.assertRaises(ValueError),
            ):
                fixture.wait_docker_startup({"pid": 7043, "uid": 0}, "engine")
            read.assert_not_called()

    def docker(self, membership=None, response=None):
        peer = {"pid": 7043, "uid": 0, "gid": 0}
        stable = {"tasks": [(7043, 100)], "fds": [0, 1, 2, 3]}

        def get(_, path, limit, deadline):
            self.assertGreater(deadline, self.clock)
            self.assertLessEqual(limit, 512 * 1024)
            if response:
                return response(path)
            if path == "/version":
                return {"ApiVersion": "1.41", "MinAPIVersion": "1.24"}
            if path == "/v1.41/info":
                return {"ID": "engine"}
            self.assertEqual(path, "/v1.41/containers/json?all=1")
            return []

        with (
            mock.patch.object(fixture, "_start", return_value=100),
            mock.patch.object(
                fixture,
                "_docker_membership",
                side_effect=lambda *_: membership(self.clock) if membership else stable,
            ),
            mock.patch.object(fixture, "_docker_json", side_effect=get) as reads,
        ):
            return fixture.wait_docker_startup(peer, "engine"), reads

    def test_docker_waits_for_natural_task_and_fd_settling_using_only_reads(self):
        def membership(now):
            return {
                "tasks": [(7043, 100)] + ([(7420, 200)] if now >= 1 else []),
                "fds": [0, 1, 2] + ([8] if now >= 2 else []),
            }

        report, reads = self.docker(membership)
        self.assertGreaterEqual(report["elapsed_seconds"], 5)
        self.assertEqual(report["membership_changes"], 2)
        self.assertGreaterEqual(report["quiet_seconds"], 3)
        self.assertEqual(report["final"]["tasks"], [(7043, 100), (7420, 200)])
        self.assertFalse(report["service_stopped_or_modified"])
        self.assertEqual(reads.call_count, 3 * report["read_passes"])

    def test_docker_persistent_churn_and_late_quiet_never_report_ready(self):
        with self.assertRaises(TimeoutError):
            self.docker(lambda now: {"tasks": [(7043, 100)], "fds": [int(now * 100)]})
        self.clock = 0
        with (
            mock.patch.object(
                fixture.time, "sleep", side_effect=lambda _: self.advance(15)
            ),
            self.assertRaises(TimeoutError),
        ):
            self.docker()

    def test_docker_changed_identity_inventory_and_invalid_version_refuse(self):
        for path, value in (
            ("/version", []),
            ("/version", {"ApiVersion": "1.40"}),
            ("/version", {"ApiVersion": "1.41", "MinAPIVersion": "1.42"}),
            ("/version", {"ApiVersion": "1.٤١"}),
            ("/v1.41/info", {"ID": "other-engine"}),
            ("/v1.41/containers/json?all=1", [{"Id": "container"}]),
        ):
            self.clock = 0

            def reply(request, path=path, value=value):
                if request == path:
                    return value
                return (
                    {"ApiVersion": "1.41"}
                    if request == "/version"
                    else {"ID": "engine"}
                )

            with self.subTest(path=path, value=value), self.assertRaises(ValueError):
                self.docker(response=reply)

    def test_docker_peer_replacement_closes_socket_without_sending_request(self):
        connection = mock.Mock()
        sock = mock.Mock()
        sock.getsockopt.return_value = fixture.struct.pack("3i", 9999, 0, 0)
        with (
            mock.patch.object(
                fixture.http.client, "HTTPConnection", return_value=connection
            ),
            mock.patch.object(fixture.socket, "socket", return_value=sock),
            self.assertRaises(ValueError),
        ):
            fixture._docker_json(
                {"pid": 7043, "uid": 0, "gid": 0}, "/version", 8192, 15
            )
        connection.request.assert_not_called()
        connection.close.assert_called_once()

    def test_docker_read_is_bounded_authenticated_and_always_closed(self):
        for status, raw in ((200, b"{}"), (503, b"{}"), (200, b"x" * 8193)):
            connection = mock.Mock()
            reply = connection.getresponse.return_value
            reply.status, reply.read.return_value = status, raw
            sock = mock.Mock()
            sock.getsockopt.return_value = fixture.struct.pack("3i", 7043, 0, 0)
            with (
                self.subTest(status=status, size=len(raw)),
                mock.patch.object(
                    fixture.http.client, "HTTPConnection", return_value=connection
                ),
                mock.patch.object(fixture.socket, "socket", return_value=sock),
            ):
                if status == 200 and len(raw) <= 8192:
                    self.assertEqual(
                        fixture._docker_json(
                            {"pid": 7043, "uid": 0, "gid": 0}, "/version", 8192, 15
                        ),
                        {},
                    )
                else:
                    with self.assertRaises(ValueError):
                        fixture._docker_json(
                            {"pid": 7043, "uid": 0, "gid": 0}, "/version", 8192, 15
                        )
            connection.request.assert_called_once_with(
                "GET", "/version", headers={"Connection": "close"}
            )
            reply.read.assert_called_once_with(8193)
            connection.close.assert_called_once()

    def test_docker_membership_rejects_parent_replacement(self):
        with (
            mock.patch.object(fixture, "_start", side_effect=[100, 200]),
            mock.patch.object(fixture, "_ids", return_value=[]),
            self.assertRaises(ValueError),
        ):
            fixture._docker_membership(7043, 100)

    def test_docker_proc_membership_rejects_excess_or_non_numeric_entries(self):
        for names in (["0", "1", "2"], ["../other"], ["4294967296"]):
            entries = mock.MagicMock()
            entries.__enter__.return_value = iter(
                SimpleNamespace(name=n) for n in names
            )
            with (
                self.subTest(names=names),
                mock.patch.object(fixture.os, "scandir", return_value=entries),
                self.assertRaises(ValueError),
            ):
                fixture._ids(Path("unused"), 2)

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
