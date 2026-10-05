"""Ensure the reference projection cannot copy runtime secrets into fixtures."""

import contextlib
import importlib.util
import io
import json
import unittest
from pathlib import Path
from unittest.mock import patch

spec = importlib.util.spec_from_file_location(
    "reference_capture", Path(__file__).with_name("capture.py")
)
capture = importlib.util.module_from_spec(spec)
spec.loader.exec_module(capture)


class ReferenceCaptureTests(unittest.TestCase):
    def test_only_whitelisted_layout_crosses_the_ssh_boundary(self):
        secret = "SYNTHETIC-secret-never-copy"
        record = {
            "Id": secret,
            "Name": secret,
            "Image": "sha256:" + "a" * 64,
            "Config": {
                "Env": [
                    "PUID=1000",
                    "PGID=1000",
                    "API_KEY=" + secret,
                    "PASSWORD=" + secret,
                ],
                "User": "",
                "Cmd": [secret],
                "Entrypoint": [secret],
                "Labels": {"private-token": secret},
                "Healthcheck": {"Test": [secret]},
            },
            "HostConfig": {
                "NetworkMode": "container:" + secret,
                "RestartPolicy": {"Name": "unless-stopped"},
                "Privileged": False,
                "PortBindings": {},
            },
            "State": {"Running": True},
            "Mounts": [
                {
                    "Type": "bind",
                    "Source": "/home/private-login/docker/navidrome",
                    "Destination": '/data"',
                    "RW": True,
                }
            ],
        }
        output = io.StringIO()
        with (
            patch.object(
                capture.subprocess,
                "check_output",
                return_value=json.dumps([record] * 7).encode(),
            ) as inspect,
            contextlib.redirect_stdout(output),
        ):
            capture.main()
        inspect.assert_called_once_with(
            ["docker", "inspect", *capture.SERVICES],
            timeout=10,
            stderr=capture.subprocess.DEVNULL,
        )
        self.assertNotIn(secret, output.getvalue())
        self.assertNotIn("private-login", output.getvalue())
        projected = json.loads(output.getvalue())
        self.assertEqual(len(projected["services"]), 7)
        service = projected["services"][0]
        self.assertEqual(service["application_owner"], {"puid": 1000, "pgid": 1000})
        self.assertEqual(service["environment_values_omitted"], 2)
        self.assertEqual(service["network"], "container:<reference-vpn>")
        self.assertEqual(service["mounts"][0]["target"], '/data"')

    def test_projection_preserves_literal_case_and_flags_unsafe_paths(self):
        self.assertEqual(capture.redact_source("/mnt/storage/TV", 0), "/mnt/storage/TV")
        self.assertEqual(capture.literal_path('/data"'), '/data"')
        for path in ["/mnt/../etc", "/mnt/data\nsecret", "relative"]:
            with self.assertRaises(ValueError):
                capture.literal_path(path)


if __name__ == "__main__":
    unittest.main()
