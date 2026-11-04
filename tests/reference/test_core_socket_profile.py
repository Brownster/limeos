"""Native profile-fixture boundaries; no installed seccomp qualification."""

import copy
import errno
import json
import sys
import unittest
from pathlib import Path
from unittest import mock

ROOT = Path(__file__).resolve().parents[2]
GUEST = ROOT / "tests/privileged_vm"
sys.path.insert(0, str(GUEST))
try:
    import p04_core_lookup_probe as probe
    import p04_core_profile_guest as guest
finally:
    sys.path.pop(0)


class CoreSocketProfile(unittest.TestCase):
    def test_rendered_matrix_preserves_every_unselected_directive(self):
        source = (
            ROOT / "tests/fixtures/automation/core-profile-baseline.service"
        ).read_text()
        for name in guest.VARIANTS:
            rendered = guest.render_unit(source, name)
            before, after = source.splitlines(), rendered.splitlines()
            self.assertEqual(len(before), len(after))
            changed = {a.partition("=")[0] for a, b in zip(before, after) if a != b}
            expected = {"ExecStart", "RuntimeDirectory", "Restart"}
            if name in ("no-suid", "both"):
                expected.add("RestrictSUIDSGID")
            if name in ("explicit", "both"):
                expected.add("SystemCallFilter")
            self.assertEqual(changed, expected)
            self.assertIn("NoNewPrivileges=yes\n", rendered)
            self.assertIn("MemoryMax=128M\n", rendered)
        with self.assertRaises(ValueError):
            guest.render_unit(source, "arbitrary")
        for changed in (
            source.replace("RestrictSUIDSGID=yes", "RestrictSUIDSGID=no"),
            source + "\nExecStart=/unexpected\n",
        ):
            with self.assertRaises(ValueError):
                guest.render_unit(changed, "baseline")

    def test_guest_and_probe_refuse_before_effects_or_syscall(self):
        with (
            mock.patch.object(
                guest.fixture, "disposable_guard", side_effect=SystemExit
            ),
            mock.patch.object(guest, "run") as run,
            self.assertRaises(SystemExit),
        ):
            guest.main()
        run.assert_not_called()
        with (
            mock.patch.object(probe.Path, "read_text", return_value="production"),
            mock.patch.object(probe, "lookup") as lookup,
            self.assertRaises(SystemExit),
        ):
            probe.main()
        lookup.assert_not_called()

    def test_fixed_lookup_retains_errno_and_closes_fd_on_fstat_failure(self):
        libc = mock.Mock()
        libc.syscall.return_value = -1
        with (
            mock.patch.object(probe.ctypes, "CDLL", return_value=libc),
            mock.patch.object(probe.ctypes, "get_errno", return_value=errno.ENOSYS),
            mock.patch.object(probe.os, "close") as close,
        ):
            self.assertEqual(probe.lookup(), {"openat2_errno": errno.ENOSYS})
            close.assert_not_called()
        arguments = libc.syscall.call_args.args
        self.assertEqual(arguments[0].value, 437)
        self.assertEqual(arguments[1].value, -100)
        self.assertEqual(
            arguments[2].value, b"/run/limeos-storage-reader/executor.sock"
        )
        libc.syscall.return_value = 17
        with (
            mock.patch.object(probe.ctypes, "CDLL", return_value=libc),
            mock.patch.object(probe.os, "fstat", side_effect=OSError("gone")),
            mock.patch.object(probe.os, "close") as close,
            self.assertRaises(OSError),
        ):
            probe.lookup()
        close.assert_called_once_with(17)

    def test_report_rejects_missing_duplicate_and_oversized_output(self):
        value = {"openat2_errno": 0}
        line = "B1_CORE_LOOKUP " + json.dumps(value)
        self.assertEqual(guest.parse_report("Starting unit\n" + line + "\n"), value)
        for text in ("", line + "\n" + line, "x" * 65537):
            with self.assertRaises(ValueError):
                guest.parse_report(text)

    def test_matrix_refuses_wrong_errno_identity_privileges_and_limits(self):
        socket = {"inode": 123, "uid": 0}
        value = {
            "core_uid": 104,
            "rpc_gid": 112,
            "supplementary_gids": [113, 114],
            "socket": socket,
            "control": {"openat2_errno": 0, "uid": 104, "socket": socket},
            "variants": {},
        }
        for name, (suid, _explicit) in guest.VARIANTS.items():
            value["variants"][name] = {
                "probe": {
                    "uid": 104,
                    "gid": 112,
                    "groups": [112, 113, 114],
                    "openat2_errno": errno.ENOSYS if suid else 0,
                    "socket": socket,
                    "status": {
                        "NoNewPrivs": "1",
                        "Seccomp": "2",
                        "CapEff": "0000000000000000",
                        "CapPrm": "0000000000000000",
                        "CapBnd": "0000000000000000",
                    },
                    "nofile": [1024, 1024],
                },
                "properties": {
                    "MemoryMax": "134217728",
                    "TasksMax": "32",
                    "LimitNOFILE": "1024",
                    "RestrictSUIDSGID": "yes" if suid else "no",
                    "Result": "success",
                    "ExecMainStatus": "0",
                },
            }
        guest.validate_matrix(value)
        for section, key, wrong in (
            ("probe", "uid", 0),
            ("probe", "openat2_errno", errno.ENOSYS),
            ("probe", "socket", {"inode": 124}),
            ("probe", "nofile", [4096, 4096]),
            ("properties", "MemoryMax", "268435456"),
        ):
            bad = copy.deepcopy(value)
            bad["variants"]["both"][section][key] = wrong
            with self.subTest(key=key), self.assertRaises(ValueError):
                guest.validate_matrix(bad)


if __name__ == "__main__":
    unittest.main()
