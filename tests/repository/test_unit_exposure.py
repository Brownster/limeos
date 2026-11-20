"""Unit hardening contract and the exposure gate in scripts/check_unit_exposure.py."""

import importlib.util
import re
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
UNITS = ROOT / "packaging/systemd"
SCRIPT = ROOT / "scripts/check_unit_exposure.py"
spec = importlib.util.spec_from_file_location("check_unit_exposure", SCRIPT)
gate = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gate)


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


def offline_analysis():
    """True when systemd-analyze can score unit files offline (systemd 250+)."""
    if shutil.which("systemd-analyze") is None:
        return False
    version = subprocess.run(
        ["systemd-analyze", "--version"], capture_output=True, text=True, check=False
    ).stdout
    found = re.match(r"systemd (\d+)", version)
    return bool(found) and int(found[1]) >= 250


class HostObservationRunsUnprivileged(unittest.TestCase):
    def test_storaged_has_a_dedicated_account_and_no_capability(self):
        values = service("limeos-storaged.service")
        self.assertEqual(values["User"], ["limeos-storaged"])
        self.assertEqual(values["Group"], ["limeos-host-access"])
        self.assertEqual(values["CapabilityBoundingSet"], [""])
        self.assertEqual(values["AmbientCapabilities"], [""])
        for key, value in {
            "NoNewPrivileges": "yes",
            "ProtectSystem": "strict",
            "ProtectHome": "yes",
            "PrivateTmp": "yes",
            "PrivateDevices": "yes",
            "PrivateNetwork": "yes",
            "ProtectClock": "yes",
            "ProtectHostname": "yes",
            "RestrictNamespaces": "yes",
            "IPAddressDeny": "any",
            "RemoveIPC": "yes",
        }.items():
            self.assertEqual(values[key], [value], key)
        self.assertEqual(
            values["SystemCallFilter"], ["@system-service", "~@privileged @resources"]
        )

    def test_package_hands_the_host_ceiling_to_the_service_group_on_every_configure(self):
        """An upgrade keeps a root:root 0600 host.json; the new user must read it."""
        script = (ROOT / "packaging/debian/postinst").read_text()
        function = re.search(r"^fct_reader_access\(\) \{\n.*?^\}\n", script, re.M | re.S)
        self.assertIsNotNone(function, "postinst defines fct_reader_access")
        group = service("limeos-storaged.service")["Group"][0]
        call = f"fct_reader_access /etc/limeos/system-policy/host.json {group}"
        self.assertIn(call, script)
        # After every ceiling is validated or created, before any restart, and
        # after both directories above it are forced to 0755 (search for all).
        loop_end = script.index("done", script.index("for target in host container storage"))
        position = script.index(call)
        self.assertLess(loop_end, position)
        self.assertLess(position, script.index("systemctl restart"))
        for directory in ("/etc/limeos root 0755", "/etc/limeos/system-policy root 0755"):
            self.assertLess(script.index(f"fct_directory {directory}"), position)
        with tempfile.TemporaryDirectory() as directory:
            work = Path(directory)
            calls = work / "calls"
            ceiling = work / "host.json"
            ceiling.write_text("{}")
            ceiling.chmod(0o600)
            link = work / "link.json"
            link.symlink_to(ceiling)
            program = (
                f"chown() {{ printf '%s\\n' \"$*\" >>{calls}; }}\n"
                + function[0]
                + f"fct_reader_access {ceiling} {group}\n"
                + f"if fct_reader_access {link} {group}; then exit 9; fi\n"
                + f"if fct_reader_access {work}/missing.json {group}; then exit 9; fi\n"
            )
            result = subprocess.run(
                ["/bin/bash", "--noprofile", "--norc", "-c", program],
                capture_output=True,
                text=True,
                timeout=10,
                check=False,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(calls.read_text(), f"root:{group} {ceiling}\n")
            self.assertEqual(ceiling.stat().st_mode & 0o7777, 0o640)
            self.assertIn("Refusing non-regular ceiling", result.stderr)

    def test_core_reaches_the_runtime_directory_and_socket_through_the_service_group(self):
        storaged = service("limeos-storaged.service")
        self.assertEqual(storaged["RuntimeDirectory"], ["limeos-storaged"])
        self.assertEqual(storaged["RuntimeDirectoryMode"], ["0750"])
        self.assertNotIn("StateDirectory", storaged)
        core = service("limeos-core.service")
        self.assertIn(storaged["Group"][0], core["SupplementaryGroups"][0].split())

    def test_package_creates_the_account_without_docker_access(self):
        script = (ROOT / "packaging/debian/postinst").read_text()
        accounts = re.search(r"for account in ([^;]+); do fct_account", script)[1]
        self.assertIn("limeos-storaged", accounts.split())
        # The shared account check refuses Docker for everyone but containerd.
        self.assertIn('"${account}" != limeos-containerd', script)


class RootStorageUnitsSayWhatRemains(unittest.TestCase):
    ROOT_UNITS = [
        "limeos-storage-reader.service",
        "limeos-storage-targets.service",
        "limeos-storage-ready.service",
        "limeos-storage-mounts.service",
    ]

    def test_namespace_free_settings_apply_to_every_root_storage_unit(self):
        for name in self.ROOT_UNITS:
            with self.subTest(unit=name):
                values = service(name)
                self.assertEqual(values["User"], ["root"])
                self.assertEqual(values["NoNewPrivileges"], ["yes"])
                self.assertEqual(values["RestrictNamespaces"], ["yes"])
                self.assertEqual(values["IPAddressDeny"], ["any"])
                # Host mount identity: no setting that clones the mount namespace.
                for key in ("ProtectSystem", "ProtectHome", "PrivateTmp"):
                    self.assertEqual(values.get(key, ["no"]), ["no"], key)

    def test_read_and_wait_roles_deny_privileged_syscalls(self):
        self.assertEqual(
            service("limeos-storage-reader.service")["SystemCallFilter"],
            ["@system-service openat2", "~@privileged"],
        )
        self.assertEqual(
            service("limeos-storage-ready.service")["SystemCallFilter"],
            ["@system-service", "~@privileged @resources"],
        )
        self.assertEqual(
            service("limeos-storage-ready.service")["RestrictAddressFamilies"],
            ["none"],
        )
        self.assertEqual(
            service("limeos-storage-targets.service")["SystemCallFilter"],
            ["@system-service", "~@resources"],
        )

    def test_root_without_capabilities_is_not_described_as_harmless(self):
        for name in self.ROOT_UNITS[:3]:
            with self.subTest(unit=name):
                text = " ".join(
                    line.removeprefix("#").strip()
                    for line in (UNITS / name).read_text().splitlines()
                    if line.startswith("#")
                )
                self.assertIn("owner write", text)
                self.assertIn("ProtectClock is off", text)

    def test_mount_unit_records_the_capability_gap(self):
        text = (UNITS / "limeos-storage-mounts.service").read_text()
        self.assertIn("# Known gap: only the guardian's fixed helper needs CAP_SYS_ADMIN", text)


STUB = """#!/bin/sh
# A stand-in for systemd-analyze: scores a unit by its "# score: N.N" line.
for unit; do :; done
value=$(sed -n 's/^# score: //p' "$unit")
[ -n "$value" ] && printf 'Overall exposure level for %s: %s OK\\n' "$(basename "$unit")" "$value"
exit 0
"""


class ExposureGate(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.work = Path(directory.name)
        self.analyze = self.work / "systemd-analyze"
        self.analyze.write_text(STUB)
        self.analyze.chmod(0o755)
        (self.work / "now").mkdir()
        (self.work / "base").mkdir()

    def units(self, where, **scores):
        for name, value in scores.items():
            text = "[Service]\n" + (f"# score: {value}\n" if value is not None else "")
            (self.work / where / f"{name}.service").write_text(text)

    def check(self, ceilings, base=True):
        return gate.check(
            self.work / "now",
            ceilings,
            self.work / "base" if base else None,
            analyze=str(self.analyze),
        )

    def test_a_unit_that_got_worse_fails_even_inside_its_ceiling(self):
        self.units("base", a="1.0")
        self.units("now", a="1.4")
        problems = self.check({"a.service": 20})
        self.assertEqual(len(problems), 1, problems)
        self.assertIn("worse than the base's 1.0", problems[0])

    def test_a_scoring_version_that_moves_every_unit_does_not_fail(self):
        # Another systemd weighs the same files 0.3 higher than the ceilings.
        self.units("base", a="2.4", b="0.7")
        self.units("now", a="2.4", b="0.7")
        self.assertEqual(self.check({"a.service": 21, "b.service": 4}), [])
        # And one that scores lower passes too, but the relative check still
        # catches a loosened unit that stays under its ceiling.
        self.units("base", a="1.0")
        self.units("now", a="1.1")
        self.assertIn("worse than the base", self.check({"a.service": 21, "b.service": 4})[0])

    def test_the_backstop_fails_above_the_ceiling_plus_margin_even_when_unchanged(self):
        self.units("base", a="2.5")
        self.units("now", a="2.5")
        problems = self.check({"a.service": 21})
        self.assertEqual(len(problems), 1, problems)
        self.assertIn("above its ceiling 2.1 plus the 0.3 margin", problems[0])
        self.assertEqual(self.check({"a.service": 22}), [])

    def test_a_unit_new_since_the_base_has_the_backstop_only(self):
        self.units("now", a="3.0")
        self.assertEqual(self.check({"a.service": 30}), [])
        self.assertIn("above its ceiling", self.check({"a.service": 20})[0])

    def test_a_unit_without_a_ceiling_or_a_score_fails(self):
        self.units("base", a="1.0", b="1.0")
        self.units("now", a="1.0", b=None)
        problems = self.check({"b.service": 20})
        self.assertEqual(len(problems), 2, problems)
        self.assertIn("a.service: no exposure ceiling recorded", problems[0])
        self.assertIn("b.service: systemd-analyze gave no score", problems[1])
        self.units("now", b="1.0")
        self.units("base", b=None)
        problems = self.check({"a.service": 20, "b.service": 20})
        self.assertEqual(len(problems), 1, problems)
        self.assertIn("gave the base no score", problems[0])

    def test_without_a_base_only_the_backstop_applies(self):
        self.units("now", a="2.0")
        self.assertEqual(self.check({"a.service": 20}, base=False), [])
        self.assertEqual(len(self.check({"a.service": 10}, base=False)), 1)

    def test_a_ceiling_for_a_missing_unit_fails(self):
        problems = self.check({"gone.service": 10})
        self.assertEqual(problems, ["gone.service: ceiling recorded for a unit that does not exist"])

    def test_base_units_are_read_from_git_at_the_revision(self):
        if shutil.which("git") is None:
            self.skipTest("needs git")
        head = gate.git("rev-parse", "HEAD")
        if head.returncode != 0:
            self.skipTest("not a git checkout")
        gate.base_units("HEAD", self.work / "base")
        names = sorted(path.name for path in (self.work / "base").glob("*.service"))
        committed = gate.git("ls-tree", "--name-only", "HEAD:packaging/systemd").stdout.split()
        self.assertEqual(names, sorted(n for n in committed if n.endswith(".service")))
        self.assertEqual(gate.resolve_base("abc123"), "abc123")

    def test_every_packaged_unit_has_a_ceiling(self):
        self.assertEqual(
            set(gate.CEILINGS), {path.name for path in UNITS.glob("*.service")}
        )


class ExposureGateWithSystemd(unittest.TestCase):
    @unittest.skipUnless(offline_analysis(), "needs systemd-analyze 250 or newer")
    def test_packaged_units_are_within_their_ceilings_and_no_worse_than_themselves(self):
        self.assertEqual(gate.check(UNITS, gate.CEILINGS, UNITS), [])

    @unittest.skipUnless(offline_analysis(), "needs systemd-analyze 250 or newer")
    def test_a_loosened_unit_fails(self):
        with tempfile.TemporaryDirectory() as directory:
            copy = Path(directory)
            for path in UNITS.glob("*.service"):
                shutil.copyfile(path, copy / path.name)
            unit = copy / "limeos-storaged.service"
            unit.write_text(unit.read_text().replace("PrivateNetwork=yes\n", ""))
            problems = gate.check(copy, gate.CEILINGS, UNITS)
        self.assertTrue(problems)
        self.assertTrue(all(p.startswith("limeos-storaged.service: exposure") for p in problems))
        self.assertTrue(any("worse than the base" in p for p in problems), problems)

    @unittest.skipUnless(offline_analysis(), "needs systemd-analyze 250 or newer")
    def test_a_unit_without_a_ceiling_fails(self):
        with tempfile.TemporaryDirectory() as directory:
            copy = Path(directory)
            shutil.copyfile(
                UNITS / "limeos-storaged.service", copy / "limeos-extra.service"
            )
            problems = gate.check(copy, {"limeos-extra.service": 100}, None)
            self.assertEqual(problems, [])
            problems = gate.check(copy, {}, None)
        self.assertEqual(len(problems), 1)
        self.assertIn("no exposure ceiling recorded", problems[0])


if __name__ == "__main__":
    unittest.main()
