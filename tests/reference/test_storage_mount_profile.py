# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Static contracts for the dormant dedicated A1 effect package profile."""

import importlib.util
import json
import re
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location(
    "storage_mount_package", ROOT / "packaging/build.py"
)
package = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(package)
UNIT = ROOT / "packaging/systemd/limeos-storage-mounts.service"


def properties():
    values = {}
    for line in UNIT.read_text().splitlines():
        if line and not line.startswith(("#", "[")):
            key, value = line.split("=", 1)
            values.setdefault(key, []).append(value)
    return values


class StorageMountProfileTests(unittest.TestCase):
    def test_unit_matches_actual_runtime_fixed_role_policy_and_socket(self):
        runtime = (ROOT / "bins/executor/src/storage_runtime.rs").read_text()

        def constant(name):
            return re.search(rf'const {name}: &str = "([^"]+)";', runtime)[1]

        values = properties()
        self.assertEqual(
            values["ExecStart"],
            [
                f"ExecStart=/usr/lib/limeos/limeos-executor {constant('POLICY')} {constant('SOCKET')} {constant('ROLE')}".removeprefix(
                    "ExecStart="
                )
            ],
        )
        guardian = (ROOT / "bins/executor/src/storage_mount_supervision.rs").read_text()
        image = re.search(r'const IMAGE: &str = "([^"]+)";', guardian)[1]
        self.assertEqual(values["ExecStart"][0].split()[0], image)
        configuration = (
            ROOT / "bins/executor/src/storage_mount_admission.rs"
        ).read_text()
        configured = set(
            re.findall(r'Path::new\("(/etc/limeos/[^"]+)"\)', configuration)
        )
        self.assertEqual(
            set(values["ConditionPathExists"]), configured | {constant("POLICY")}
        )

    def test_dedicated_role_has_exact_caps_unchanged_ceilings_and_host_namespace(self):
        values = properties()
        caps = {"CAP_DAC_READ_SEARCH", "CAP_SYS_PTRACE", "CAP_SYS_ADMIN"}
        self.assertEqual(set(values["CapabilityBoundingSet"][0].split()), caps)
        self.assertEqual(set(values["AmbientCapabilities"][0].split()), caps)
        for key, value in {
            "User": "root",
            "Group": "limeos-host-access",
            "NoNewPrivileges": "yes",
            "LimitNOFILE": "256",
            "MemoryMax": "64M",
            "TasksMax": "16",
            "MemorySwapMax": "0",
            "PrivateMounts": "no",
            "PrivateTmp": "no",
            "PrivateDevices": "no",
            "PrivateUsers": "no",
            "ProtectSystem": "no",
            "ProtectHome": "no",
            "ProtectKernelTunables": "no",
            "ProtectKernelModules": "no",
            "ProtectControlGroups": "no",
            "ProtectProc": "default",
            "ProcSubset": "all",
            "RestrictSUIDSGID": "no",
        }.items():
            self.assertEqual(values[key], [value], key)
        self.assertFalse(
            set(values)
            & {
                "BindPaths",
                "BindReadOnlyPaths",
                "ReadWritePaths",
                "ReadOnlyPaths",
                "TemporaryFileSystem",
            }
        )
        self.assertEqual(
            set(values["SystemCallFilter"][0].split()),
            {
                "@system-service",
                "openat2",
                "mount",
                "pidfd_open",
                "pidfd_send_signal",
                "prctl",
                "setsid",
                "memfd_create",
            },
        )

    def test_default_install_cannot_activate_or_provision_mount_authority(self):
        self.assertNotIn("[Install]", UNIT.read_text())
        self.assertFalse(UNIT.with_suffix(".socket").exists())
        self.assertNotIn(
            "storage-mounts", (ROOT / "packaging/debian/postinst").read_text()
        )
        self.assertNotIn(
            "storage-mounts",
            (ROOT / "packaging/systemd/limeos-core.service").read_text(),
        )
        values = properties()
        self.assertEqual(values["RuntimeDirectory"], ["limeos-storage-mounts"])
        self.assertEqual(values["RuntimeDirectoryMode"], ["0750"])
        self.assertEqual(
            set(values["ConditionPathExists"]),
            {
                "/etc/limeos/system-policy/storage-mounts.json",
                "/etc/limeos/storage-contract.json",
                "/etc/limeos/pools.json",
                "/etc/limeos/protection.json",
                "/etc/limeos/shares.json",
            },
        )
        disabled = json.loads(
            (ROOT / "packaging/config/storage-mounts.example.json").read_text()
        )
        self.assertIs(disabled["allow_mount"], False)
        self.assertEqual(disabled["managed_targets"], [])
        builder = (ROOT / "packaging/build.py").read_text()
        self.assertIn('examples = documentation / "examples"', builder)

    def test_shadow_excludes_effect_unit_and_scopes_optional_teardown(self):
        self.assertTrue(
            package.exclude_effect_unit("limeos-storage-mounts.service", True)
        )
        self.assertFalse(
            package.exclude_effect_unit("limeos-storage-mounts.service", False)
        )
        prerm = (ROOT / "packaging/debian/prerm").read_text()
        shadow = package.profile_text(prerm, True)
        self.assertIn("limeos-shadow-storage-mounts.service", shadow)
        self.assertNotIn("limeos-storage-mounts.service", shadow)
        self.assertIn(
            "/run/limeos-shadow-storage-mounts/executor.sock",
            (ROOT / "packaging/build.py").read_text(),
        )

    def test_upgrade_requires_group_stop_and_preserves_recovery_state(self):
        values = properties()
        self.assertEqual(values["KillMode"], ["control-group"])
        self.assertEqual(values["SendSIGKILL"], ["yes"])
        self.assertEqual(values["Restart"], ["no"])
        prerm = (ROOT / "packaging/debian/prerm").read_text()
        self.assertIn('systemctl stop "${unit}"', prerm)
        self.assertIn('"${state}" != inactive', prerm)
        self.assertIn('"${value}" != 0', prerm)
        self.assertIn('"${populated}" == 0', prerm)
        self.assertNotIn("rm ", prerm)
        self.assertNotIn("mounts.sqlite", prerm)

    def test_optional_teardown_refuses_uncertain_load_state_with_fake_manager(self):
        # Never execute prerm or host systemctl. Evaluate only helper definitions
        # with shell-function fakes and an empty command search path.
        definitions = (
            (ROOT / "packaging/debian/prerm").read_text().split("fct_main() {", 1)[0]
        )
        fake = r"""
systemctl() {
    printf '%s\n' "$*" >>"${TEST_LOG}"
    case "${1}:${4:-}" in
        show:LoadState)
            if [[ "${MANAGER_FAIL}" == 1 ]]; then return 92; fi
            printf '%s\n' "${LOAD_STATE}" ;;
        show:ActiveState) printf '%s\n' "${ACTIVE_STATE}" ;;
        show:ControlGroup) printf '%s\n' "${CONTROL_GROUP}" ;;
        show:KillMode) printf '%s\n' control-group ;;
        show:SendSIGKILL) printf '%s\n' yes ;;
        stop:)
            if [[ "${STOP_FAIL}" == 1 ]]; then return 93; fi ;;
        *) return 97 ;;
    esac
}
fct_optional_cgroup_state() { printf '%s\n' "${CGROUP_STATE}"; }
fct_stop_optional_service limeos-storage-mounts.service
"""
        cache = ROOT / ".cache"
        cache.mkdir(exist_ok=True)
        cases = [
            ({"LOAD_STATE": "not-found"}, True, False),
            ({"LOAD_STATE": "loaded"}, True, True),
            ({"LOAD_STATE": "masked", "ACTIVE_STATE": "active"}, False, False),
            ({"LOAD_STATE": "error"}, False, False),
            ({"LOAD_STATE": "unknown"}, False, False),
            ({"LOAD_STATE": ""}, False, False),
            ({"MANAGER_FAIL": "1"}, False, False),
            ({"LOAD_STATE": "not-found", "ACTIVE_STATE": "active"}, False, False),
            (
                {
                    "LOAD_STATE": "not-found",
                    "CONTROL_GROUP": "/system.slice/limeos-storage-mounts.service",
                },
                False,
                False,
            ),
            ({"LOAD_STATE": "not-found", "CGROUP_STATE": "present"}, False, False),
            ({"LOAD_STATE": "loaded", "STOP_FAIL": "1"}, False, True),
            ({"LOAD_STATE": "loaded", "ACTIVE_STATE": "active"}, False, True),
        ]
        with tempfile.TemporaryDirectory(dir=cache) as temporary:
            log = Path(temporary) / "manager.log"
            for changes, succeeds, stopped in cases:
                with self.subTest(changes=changes):
                    log.write_text("")
                    environment = {
                        "PATH": "/nonexistent",
                        "TEST_LOG": str(log),
                        "LOAD_STATE": "not-found",
                        "ACTIVE_STATE": "inactive",
                        "CONTROL_GROUP": "",
                        "CGROUP_STATE": "absent",
                        "MANAGER_FAIL": "0",
                        "STOP_FAIL": "0",
                        **changes,
                    }
                    result = subprocess.run(
                        ["/bin/bash", "--noprofile", "--norc", "-s"],
                        input=definitions + fake,
                        text=True,
                        capture_output=True,
                        env=environment,
                        timeout=5,
                        check=False,
                    )
                    self.assertEqual(result.returncode == 0, succeeds, result.stderr)
                    self.assertEqual(
                        "stop limeos-storage-mounts.service\n" in log.read_text(),
                        stopped,
                    )
        self.assertIn(
            'load_state="$(systemctl show "${unit}" -p LoadState --value)"', definitions
        )
        self.assertNotIn('if [[ "$(systemctl show', definitions)

    def test_core_guest_example_is_only_a_socket_merge_fragment(self):
        fragment = json.loads(
            (ROOT / "packaging/config/storage-mounts-core.example.json").read_text()
        )
        self.assertEqual(
            fragment,
            {"storage_mount_socket": "/run/limeos-storage-mounts/executor.sock"},
        )

    def test_package_guard_rejects_writable_code_and_mutable_state_exec(self):
        cache = ROOT / ".cache"
        cache.mkdir(exist_ok=True)
        with tempfile.TemporaryDirectory(dir=cache) as temporary:
            stage = Path(temporary)
            code = stage / "usr/lib/limeos"
            code.mkdir(parents=True)
            for name in [
                "limeos-core",
                "limeos-executor",
                "limeosctl",
                "limeos-password-worker",
            ]:
                (code / name).write_bytes(b"fixture executable")
                (code / name).chmod(0o755)
            units = stage / "lib/systemd/system"
            units.mkdir(parents=True)
            unit = units / UNIT.name
            unit.write_text(UNIT.read_text())
            unit.chmod(0o644)
            package.check_root_execution_payload(stage, "limeos")
            (code / "limeos-executor").chmod(0o775)
            with self.assertRaises(SystemExit):
                package.check_root_execution_payload(stage, "limeos")
            (code / "limeos-executor").chmod(0o755)
            unit.write_text(
                unit.read_text().replace(
                    "ExecStart=/usr/lib/limeos/limeos-executor",
                    "ExecStart=/var/lib/limeos/core/limeos-executor",
                )
            )
            with self.assertRaises(SystemExit):
                package.check_root_execution_payload(stage, "limeos")


if __name__ == "__main__":
    unittest.main()
