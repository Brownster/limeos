"""Package contract for protected stack reads; installed syscall tests are separate."""

import importlib.util
import json
import subprocess
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
UNIT = ROOT / "packaging/systemd/limeos-storage-reader.service"
SPEC = importlib.util.spec_from_file_location(
    "stack_reader_package", ROOT / "packaging/build.py"
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


class StackReaderProfileTests(unittest.TestCase):
    def test_new_storage_policy_adds_only_closed_read_permissions(self):
        script = (ROOT / "packaging/debian/postinst").read_text()
        line = next(
            line.strip()
            for line in script.splitlines()
            if '"allow_container_health_read":true' in line
        )
        suffix = '>"${temporary}"'
        self.assertTrue(line.endswith(suffix))
        # Execute the actual printf using Bash builtins, without its file redirect
        # or any account/systemd/filesystem operations from the package script.
        result = subprocess.run(
            [
                "/bin/bash",
                "--noprofile",
                "--norc",
                "-c",
                "core_uid=1001\n" + line.removesuffix(suffix),
            ],
            env={"PATH": "/nonexistent"},
            text=True,
            capture_output=True,
            timeout=5,
            check=True,
        )
        self.assertEqual(
            json.loads(result.stdout),
            {
                "version": 1,
                "core_uid": 1001,
                "allow_health": True,
                "allow_storage_read": True,
                "allow_stack_read": True,
                "allow_container_health_read": True,
                "managed_containers": [],
            },
        )
        preserved = script[
            script.index(
                'if [[ -e "/etc/limeos/system-policy/${target}.json"'
            ) : script.index('temporary="$(mktemp')
        ]
        self.assertIn("check-ceiling", preserved)
        self.assertIn("continue", preserved)
        self.assertNotIn("allow_container_health_read", preserved)

    def test_protected_open_is_compatible_with_one_read_capability_and_no_larger_limits(self):
        for shadow in (False, True):
            with self.subTest(shadow=shadow):
                values = properties(package.profile_text(UNIT.read_text(), shadow))
                for key, expected in {
                    "User": "limeos-shadow-storage-reader" if shadow else "limeos-storage-reader",
                    "SupplementaryGroups": "docker",
                    "NoNewPrivileges": "yes",
                    "CapabilityBoundingSet": "CAP_DAC_READ_SEARCH",
                    "AmbientCapabilities": "CAP_DAC_READ_SEARCH",
                    "RestrictSUIDSGID": "no",
                    "PrivateMounts": "no",
                    "PrivateDevices": "no",
                    "RestrictAddressFamilies": "AF_UNIX",
                    "SystemCallArchitectures": "native",
                    "MemoryMax": "64M",
                    "MemorySwapMax": "0",
                    "LimitNOFILE": "256",
                    "TasksMax": "16",
                    "CPUQuota": "50%",
                }.items():
                    self.assertEqual(values[key], [expected], key)
                # openat2 for protected reads; privileged calls removed after.
                self.assertEqual(
                    values["SystemCallFilter"],
                    ["@system-service openat2", "~@privileged"],
                )
                self.assertEqual(values["RestrictNamespaces"], ["yes"])
                self.assertEqual(values["IPAddressDeny"], ["any"])
                self.assertFalse(
                    set(values)
                    & {
                        "RootDirectory",
                        "RootImage",
                        "BindPaths",
                        "BindReadOnlyPaths",
                        "TemporaryFileSystem",
                        "ReadOnlyPaths",
                        "ReadWritePaths",
                    }
                )

    def test_standard_role_has_only_fixed_protected_policy_socket_and_executable(self):
        values = properties(UNIT.read_text())
        self.assertEqual(
            values["ExecStart"],
            [
                "/usr/lib/limeos/limeos-executor /etc/limeos/system-policy/storage.json /run/limeos-storage-reader/executor.sock storage"
            ],
        )
        self.assertEqual(values["Group"], ["limeos-host-access"])
        self.assertEqual(values["RuntimeDirectory"], ["limeos-storage-reader"])
        self.assertEqual(values["RuntimeDirectoryMode"], ["0750"])
        self.assertEqual(values["UMask"], ["0077"])

    def test_shadow_keeps_the_same_read_role_and_separate_fixed_paths(self):
        values = properties(package.profile_text(UNIT.read_text(), True))
        self.assertEqual(
            values["ExecStart"],
            [
                "/usr/lib/limeos-shadow/limeos-executor /etc/limeos-shadow/system-policy/storage.json /run/limeos-shadow-storage-reader/executor.sock storage"
            ],
        )
        self.assertEqual(values["Group"], ["limeos-shadow-host-access"])
        self.assertEqual(values["RuntimeDirectory"], ["limeos-shadow-storage-reader"])

    def test_installation_cannot_enable_or_start_the_reader(self):
        self.assertNotIn("[Install]", UNIT.read_text())
        self.assertFalse(UNIT.with_suffix(".socket").exists())
        # postinst names the reader's unit only on its restart allowlist,
        # where it starts it again after an upgrade or abort when prerm
        # recorded it running (PR-24, tests/repository/test_maintainer_scripts.py),
        # in the map from a refused ceiling to the unit it keeps from
        # starting, and in the guard that leaves a refused storage.json alone.
        # Its account is created with the others (PR-32). Nothing else.
        lines = [
            line.strip()
            for line in (ROOT / "packaging/debian/postinst").read_text().splitlines()
            if "storage-reader" in line and not line.lstrip().startswith("#")
        ]
        self.assertEqual(len(lines), 4, lines)
        self.assertTrue(lines[0].startswith("RESTARTED_UNITS=("))
        self.assertEqual(lines[1], "storage) printf '%s\\n' limeos-storage-reader.service ;;")
        self.assertTrue(lines[2].startswith("for account in ") and lines[2].endswith('do fct_account "${account}"; done'))
        self.assertEqual(lines[3], 'if [[ " ${refused[*]-} " != *" limeos-storage-reader.service "* ]]; then')
        self.assertFalse([line for line in lines if "systemctl" in line and "enable" in line])
        self.assertNotIn(
            "storage-reader",
            (ROOT / "packaging/systemd/limeos-core.service").read_text(),
        )


if __name__ == "__main__":
    unittest.main()
