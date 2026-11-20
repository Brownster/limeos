"""Which LimeOS units carry a systemd watchdog, and why the others do not.

A watchdog belongs only on a unit whose keep-alive thread never does work
that can legitimately block for long: there it proves the loop is alive.
On a unit whose loop thread runs device writes (fsync), a slow disk would
get a healthy operation aborted, so those units have none; each unit file
records the reasoning next to the setting.
"""

import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
UNITS = ROOT / "packaging/systemd"


def service(name):
    values = {}
    section = None
    for line in (UNITS / name).read_text().splitlines():
        if line.startswith("["):
            section = line
        elif line and not line.startswith("#") and section == "[Service]":
            key, value = line.split("=", 1)
            values.setdefault(key, []).append(value)
    return values


class ServiceWatchdogs(unittest.TestCase):
    def test_only_units_whose_loop_never_blocks_on_a_device_have_a_watchdog(self):
        expected = {
            "limeos-core.service": True,
            "limeos-storaged.service": True,
            "limeos-storage-reader.service": True,
            # Receipt-store and target-preparation fsyncs run on the loop.
            "limeos-containerd.service": False,
            "limeos-storage-targets.service": False,
            "limeos-storage-mounts.service": False,
        }
        for name, watched in expected.items():
            with self.subTest(unit=name):
                values = service(name)
                self.assertEqual("WatchdogSec" in values, watched)
                if watched:
                    self.assertEqual(values["WatchdogSec"], ["30s"])
                    self.assertEqual(values["Restart"], ["on-failure"])
                text = (UNITS / name).read_text()
                # The reasoning is recorded beside the setting either way.
                self.assertIn("No watchdog" if not watched else "Longest legitimate", text)


if __name__ == "__main__":
    unittest.main()
