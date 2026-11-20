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


class ExposureGate(unittest.TestCase):
    def test_every_packaged_unit_has_a_ceiling(self):
        self.assertEqual(
            set(gate.CEILINGS), {path.name for path in UNITS.glob("*.service")}
        )

    @unittest.skipUnless(offline_analysis(), "needs systemd-analyze 250 or newer")
    def test_packaged_units_are_within_their_ceilings(self):
        self.assertEqual(gate.check(UNITS, gate.CEILINGS), [])

    @unittest.skipUnless(offline_analysis(), "needs systemd-analyze 250 or newer")
    def test_a_loosened_unit_fails(self):
        with tempfile.TemporaryDirectory() as directory:
            copy = Path(directory)
            for path in UNITS.glob("*.service"):
                shutil.copyfile(path, copy / path.name)
            unit = copy / "limeos-storaged.service"
            unit.write_text(unit.read_text().replace("PrivateNetwork=yes\n", ""))
            problems = gate.check(copy, gate.CEILINGS)
        self.assertEqual(len(problems), 1)
        self.assertIn("limeos-storaged.service: exposure", problems[0])
        self.assertIn("above its ceiling", problems[0])

    @unittest.skipUnless(offline_analysis(), "needs systemd-analyze 250 or newer")
    def test_a_unit_without_a_ceiling_fails(self):
        with tempfile.TemporaryDirectory() as directory:
            copy = Path(directory)
            shutil.copyfile(
                UNITS / "limeos-storaged.service", copy / "limeos-extra.service"
            )
            problems = gate.check(copy, {"limeos-extra.service": 100})
            self.assertEqual(problems, [])
            problems = gate.check(copy, {})
        self.assertEqual(len(problems), 1)
        self.assertIn("no exposure ceiling recorded", problems[0])


if __name__ == "__main__":
    unittest.main()
