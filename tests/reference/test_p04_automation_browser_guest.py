"""Boundary checks for the installed browser fixture; no browser acceptance."""

import base64
import importlib.util
import io
import json
import sys
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path
from unittest import mock

SOURCE = (
    Path(__file__).resolve().parents[1]
    / "privileged_vm/p04_automation_browser_guest.py"
)
SPEC = importlib.util.spec_from_file_location("b1_browser_fixture", SOURCE)
guest = importlib.util.module_from_spec(SPEC)
sys.path.insert(0, str(SOURCE.parent))
try:
    SPEC.loader.exec_module(guest)
finally:
    sys.path.pop(0)


class BrowserFixtureBoundaries(unittest.TestCase):
    def test_network_evidence_discards_headers_bodies_and_query_secrets(self):
        driver = mock.Mock()

        def event(method, params):
            return {
                "message": json.dumps({"message": {"method": method, "params": params}})
            }

        driver.get_log.return_value = [
            event(
                "Network.requestWillBeSent",
                {
                    "requestId": "1",
                    "request": {
                        "url": "https://localhost/api/v1/auth/login?secret=private",
                        "method": "POST",
                        "headers": {"Cookie": "private"},
                        "postData": "private",
                    },
                },
            ),
            event(
                "Network.responseReceived",
                {
                    "requestId": "1",
                    "response": {"status": 200, "headers": {"Set-Cookie": "private"}},
                },
            ),
        ]
        network = guest.NetworkEvidence()
        network.drain(driver)
        self.assertEqual(
            network.responses,
            [{"method": "POST", "path": "/api/v1/auth/login", "status": 200}],
        )
        self.assertNotIn("private", json.dumps(network.__dict__))
        self.assertEqual(network.statuses("/api/v1/auth/login"), [200])

    def test_production_or_nonroot_refuses_before_any_command(self):
        for uid, hostname in ((1000, "limeos-p01-test"), (0, "hollys-pi")):
            with (
                self.subTest(uid=uid, hostname=hostname),
                mock.patch.object(guest.fixture.os, "geteuid", return_value=uid),
                mock.patch.object(guest.Path, "read_text", return_value=hostname),
                mock.patch.object(guest, "run") as command,
                mock.patch.object(guest, "oom_events") as events,
                self.assertRaises(SystemExit),
            ):
                guest.main()
            command.assert_not_called()
            events.assert_not_called()

    def test_wait_keeps_original_deadline(self):
        observe = mock.Mock(return_value=None)
        with (
            mock.patch.object(guest.time, "monotonic", side_effect=[0, 0, 1, 2]),
            mock.patch.object(guest.time, "sleep") as sleep,
            self.assertRaises(AssertionError),
        ):
            guest.wait_for(observe, timeout=2)
        self.assertEqual(observe.call_count, 1)
        sleep.assert_called_once_with(0.2)

    def test_password_blocks_capture_before_pixels_are_read(self):
        driver = mock.Mock()
        driver.find_elements.return_value = [mock.Mock()]
        driver.find_elements.return_value[0].get_attribute.return_value = "private"
        with self.assertRaises(ValueError):
            guest.screenshot(driver, {}, "forbidden")
        driver.get_screenshot_as_png.assert_not_called()

    def test_screenshot_bound_and_exact_roundtrip(self):
        driver = mock.Mock()
        driver.find_elements.return_value = []
        driver.get_screenshot_as_png.return_value = b"fixture pixels"
        evidence = {}
        guest.screenshot(driver, evidence, "safe")
        value = evidence["screenshots"]["safe"]
        self.assertEqual(base64.b64decode(value["png_base64"]), b"fixture pixels")
        self.assertEqual(len(value["sha256"]), 64)
        driver.get_screenshot_as_png.return_value = b"x" * (512 * 1024 + 1)
        with self.assertRaises(ValueError):
            guest.screenshot(driver, evidence, "large")
        self.assertNotIn("large", evidence["screenshots"])

    def test_oom_observer_keeps_only_explicit_counters(self):
        path = mock.Mock()
        path.read_text.return_value = (
            "low 1\nhigh 2\nmax 3\noom 0\noom_kill 0\noom_group_kill 0\n"
        )
        self.assertEqual(guest.oom_events(path), {"oom_kill": 0})
        with mock.patch.object(
            guest.Path, "read_text", return_value="oom_kill 0\n"
        ) as read:
            self.assertEqual(guest.oom_events(), {"oom_kill": 0})
        read.assert_called_once_with()
        path.read_text.return_value = "pgfault 100\n"
        with self.assertRaises(ValueError):
            guest.oom_events(path)

    def test_original_failure_is_persisted_without_exception_secrets(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "result.json"
            stdout = io.StringIO()
            with (
                mock.patch.object(guest.fixture, "disposable_guard"),
                mock.patch.object(guest, "oom_events", return_value={"oom": 0}),
                mock.patch.object(
                    guest, "run", side_effect=RuntimeError("private password or cookie")
                ),
                mock.patch.object(sys, "argv", [str(SOURCE), directory, str(output)]),
                redirect_stdout(stdout),
                self.assertRaises(RuntimeError),
            ):
                guest.main()
            result = output.read_text()
            self.assertEqual(json.loads(result)["summary"], "fail")
            self.assertEqual(json.loads(result)["stage"], "install")
            self.assertNotIn("private password", result + stdout.getvalue())
            self.assertEqual(json.loads(result)["error_type"], "RuntimeError")


if __name__ == "__main__":
    unittest.main()
