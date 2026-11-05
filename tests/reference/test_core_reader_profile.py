"""Core package profile contract after the measured protected-lookup conflict."""

import importlib.util
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location(
    "core_reader_package", ROOT / "packaging/build.py"
)
package = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(package)


def properties(text):
    values = {}
    for line in text.splitlines():
        if line and not line.startswith(("#", "[")):
            key, value = line.split("=", 1)
            values.setdefault(key, []).append(value)
    return values


class CoreReaderProfile(unittest.TestCase):
    def test_only_measured_syscall_controls_change_from_the_frozen_profile(self):
        baseline = properties(
            (
                ROOT / "tests/fixtures/automation/core-profile-baseline.service"
            ).read_text()
        )
        actual = properties(
            (ROOT / "packaging/systemd/limeos-core.service").read_text()
        )
        expected = dict(
            baseline,
            RestrictSUIDSGID=["no"],
            SystemCallFilter=["@system-service openat2"],
        )
        self.assertEqual(actual, expected)

    def test_standard_and_shadow_keep_identity_confinement_resources_and_separate_state(
        self,
    ):
        source = (ROOT / "packaging/systemd/limeos-core.service").read_text()
        for shadow in (False, True):
            with self.subTest(shadow=shadow):
                prefix = "limeos-shadow" if shadow else "limeos"
                value = properties(package.profile_text(source, shadow))
                for key, expected in {
                    "User": prefix + "-core",
                    "Group": prefix + "-rpc",
                    "SupplementaryGroups": prefix
                    + "-host-access "
                    + prefix
                    + "-container-access",
                    "NoNewPrivileges": "yes",
                    "CapabilityBoundingSet": "",
                    "AmbientCapabilities": "",
                    "ProtectProc": "invisible",
                    "ProtectSystem": "strict",
                    "ProtectHome": "yes",
                    "PrivateTmp": "yes",
                    "PrivateDevices": "yes",
                    "ReadWritePaths": "/var/lib/" + prefix + "/core",
                    "MemoryMax": "128M",
                    "MemorySwapMax": "0",
                    "TasksMax": "32",
                    "LimitNOFILE": "1024",
                    "CPUQuota": "200%",
                    "MemoryDenyWriteExecute": "yes",
                    "RestrictSUIDSGID": "no",
                    "SystemCallFilter": "@system-service openat2",
                    "ExecStart": "/usr/lib/" + prefix + "/limeos-core",
                }.items():
                    self.assertEqual(value[key], [expected], key)


if __name__ == "__main__":
    unittest.main()
