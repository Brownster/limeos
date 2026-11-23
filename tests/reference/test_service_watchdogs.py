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


    def test_core_is_ready_when_it_says_so_and_executors_start_at_exec(self):
        core = service("limeos-core.service")
        self.assertEqual(core["Type"], ["notify"])
        self.assertEqual(core["NotifyAccess"], ["main"])
        # The base start timeout; core extends it while a schema upgrade runs.
        self.assertEqual(core["TimeoutStartSec"], ["90s"])
        for name in ("limeos-storaged.service", "limeos-storage-reader.service"):
            with self.subTest(unit=name):
                values = service(name)
                self.assertEqual(values["Type"], ["exec"])
                self.assertNotIn("NotifyAccess", values)
                self.assertIn("Type=exec arms the watchdog at exec", (UNITS / name).read_text())


def unit_section(name):
    values = {}
    section = None
    for line in (UNITS / name).read_text().splitlines():
        if line.startswith("["):
            section = line
        elif line and not line.startswith("#") and section == "[Unit]":
            key, value = line.split("=", 1)
            values.setdefault(key, []).append(value)
    return values


def seconds(value):
    units = {"s": 1, "min": 60, "h": 3600}
    for suffix, factor in sorted(units.items(), key=lambda kv: -len(kv[0])):
        if value.endswith(suffix):
            return int(value[: -len(suffix)]) * factor
    return int(value)


class StartLimit(unittest.TestCase):
    def test_core_stops_retrying_a_start_that_keeps_failing_even_late(self):
        unit = unit_section("limeos-core.service")
        core = service("limeos-core.service")
        burst = int(unit["StartLimitBurst"][0])
        interval = seconds(unit["StartLimitIntervalSec"][0])
        restart = seconds(core["RestartSec"][0])
        # The longest a start can take: core extends its start timeout for
        # at most 30 minutes, asking a minute at a time
        # (`StartExtension::from_env`).
        longest = 30 * 60 + 60
        self.assertEqual(burst, 10)
        # More than `burst` starts must fit in the interval however late
        # each one fails, or a late-failing upgrade is retried for ever.
        self.assertLessEqual(burst * (longest + restart), interval)
        # A fast-failing start (well under a second) trips it in under a
        # minute: the burst at RestartSec apart.
        self.assertLess(burst * (restart + 1), 60)

    def test_only_core_has_a_start_limit(self):
        for unit in sorted(UNITS.glob("*.service")):
            if unit.name == "limeos-core.service":
                continue
            with self.subTest(unit=unit.name):
                values = unit_section(unit.name)
                self.assertNotIn("StartLimitBurst", values)
                self.assertNotIn("StartLimitIntervalSec", values)


if __name__ == "__main__":
    unittest.main()
