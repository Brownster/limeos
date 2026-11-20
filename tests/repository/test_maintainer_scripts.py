"""Stub-driven tests for packaging/debian/{prerm,postrm,postinst} (PR-24).

No real systemctl is ever run and no host path is touched: every script runs
with PATH set to a sandbox holding a stub systemctl and a few named coreutils,
and with DPKG_ROOT (which dpkg itself sets for maintainer scripts) pointing at
a temp root, under which the scripts find run/systemd/system, the cgroup tree,
the enablement links and their own restart record.

Run: python3 -m unittest discover -s tests/repository -p 'test_maintainer_scripts.py'
"""

import importlib.util
import re
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
DEBIAN = ROOT / "packaging/debian"
SYSTEMD = ROOT / "packaging/systemd"

CORE = "limeos-core.service"
EXECUTORS = ["limeos-containerd.service", "limeos-storaged.service"]
ENABLED = [CORE, *EXECUTORS]
READER = "limeos-storage-reader.service"
READY = "limeos-storage-ready.service"
TARGETS = "limeos-storage-targets.service"
MOUNTS = "limeos-storage-mounts.service"
OPTIONAL = [MOUNTS, READER, READY, TARGETS]
RECORD = "run/limeos-package/units"
WANTS = "etc/systemd/system/multi-user.target.wants"
# What remove and purge must leave: authority, executor journals, ceilings,
# operator configuration (dpkg itself deletes the conffile on purge).
DATA = [
    "var/lib/limeos/core/authority.sqlite",
    "var/lib/limeos/assistant/state.json",
    "var/lib/limeos/executors/container/receipts.sqlite",
    "var/lib/limeos/executors/storage/mounts.sqlite",
    "etc/limeos/core.json",
    "etc/limeos/system-policy/host.json",
    "etc/limeos/system-policy/container.json",
    "etc/limeos/system-policy/storage.json",
    "etc/limeos/storage-contract.json",
    "etc/systemd/system/limeos-core.service.d/override.conf",
]
TOOLS = ["mkdir", "mktemp", "mv", "rm"]

STUB = r"""#!/bin/bash
# A model of systemctl over $STUB_STATE/units/UNIT.{load,active,cgroup}.
set -u
state="${STUB_STATE}"
call="$*"
printf '%s\n' "${call}" >>"${state}/calls.log"
if [[ -f "${state}/fail" ]]; then
    while IFS= read -r line; do
        if [[ "${line}" == "${call}" ]]; then printf 'stub: injected failure: %s\n' "${call}" >&2; exit 1; fi
    done <"${state}/fail"
fi
get() { local value; if [[ -f "${state}/units/${1}.${2}" ]]; then read -r value <"${state}/units/${1}.${2}" || true; printf '%s\n' "${value:-}"; else printf '%s\n' "${3}"; fi; }
put() { printf '%s\n' "${3}" >"${state}/units/${1}.${2}"; }
verb="${1}"; shift
case "${verb}" in
    show)
        unit="${1}" property=''
        while (( $# )); do if [[ "${1}" == -p ]]; then property="${2}"; shift; fi; shift; done
        case "${property}" in
            LoadState) get "${unit}" load not-found ;;
            ActiveState) get "${unit}" active inactive ;;
            ControlGroup) get "${unit}" cgroup '' ;;
            KillMode) printf '%s\n' control-group ;;
            SendSIGKILL) printf '%s\n' yes ;;
            *) exit 97 ;;
        esac ;;
    stop)
        for unit in "$@"; do
            # As systemd: a unit with no file and no work is not loaded.
            if [[ "$(get "${unit}" load not-found)" == not-found && "$(get "${unit}" active inactive)" == inactive ]]; then
                printf 'Failed to stop %s: Unit %s not loaded.\n' "${unit}" "${unit}" >&2; exit 5
            fi
            put "${unit}" active inactive
        done ;;
    start|restart)
        for unit in "$@"; do
            case "$(get "${unit}" load not-found)" in
                loaded) put "${unit}" active active ;;
                masked) printf 'Unit %s is masked.\n' "${unit}" >&2; exit 1 ;;
                *) printf 'Unit %s not found.\n' "${unit}" >&2; exit 5 ;;
            esac
        done ;;
    enable|disable|daemon-reload) ;;
    *) exit 97 ;;
esac
"""


def load_build():
    spec = importlib.util.spec_from_file_location("package_build", ROOT / "packaging/build.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class Sandbox:
    """A temp root, a stub manager and a PATH holding nothing else."""

    def __init__(self, base: Path, systemd=True, systemctl=True):
        self.root = base / "root"
        self.bin = base / "bin"
        self.state = base / "state"
        for directory in (self.root, self.bin, self.state / "units"):
            directory.mkdir(parents=True)
        (self.state / "calls.log").write_text("")
        for tool in TOOLS:
            found = shutil.which(tool, path="/usr/bin:/bin")
            assert found, tool
            (self.bin / tool).symlink_to(found)
        if systemctl:
            stub = self.bin / "systemctl"
            stub.write_text(STUB)
            stub.chmod(0o755)
        if systemd:
            (self.root / "run/systemd/system").mkdir(parents=True)
        cgroup = self.root / "sys/fs/cgroup"
        cgroup.mkdir(parents=True)
        (cgroup / "cgroup.controllers").write_text("cpu memory pids\n")
        for path in DATA:
            target = self.root / path
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text(path)

    def unit(self, name, load="loaded", active="inactive"):
        (self.state / f"units/{name}.load").write_text(load + "\n")
        (self.state / f"units/{name}.active").write_text(active + "\n")

    def active(self, name):
        path = self.state / f"units/{name}.active"
        return path.read_text().strip() if path.exists() else "inactive"

    def installed(self, running=(), enabled=True):
        for name in [*ENABLED, *OPTIONAL]:
            self.unit(name, active="active" if name in running else "inactive")
        if enabled:
            wants = self.root / WANTS
            wants.mkdir(parents=True, exist_ok=True)
            for name in ENABLED:
                (wants / name).symlink_to(f"/lib/systemd/system/{name}")

    def fail(self, *calls):
        (self.state / "fail").write_text("".join(f"{call}\n" for call in calls))

    def calls(self):
        return (self.state / "calls.log").read_text().splitlines()

    def mutating(self):
        return [c for c in self.calls() if not c.startswith("show ")]

    def clear(self):
        (self.state / "calls.log").write_text("")

    def record(self):
        path = self.root / RECORD
        return path.read_text().splitlines() if path.exists() else None

    def run(self, script, *args, source=None):
        program = source if source is not None else (DEBIAN / script).read_text()
        return subprocess.run(
            ["/bin/bash", "--noprofile", "--norc", "-s", "--", *args],
            input=program,
            text=True,
            capture_output=True,
            env={
                "PATH": str(self.bin),
                "DPKG_ROOT": str(self.root),
                "STUB_STATE": str(self.state),
                "LC_ALL": "C",
            },
            timeout=30,
            check=False,
        )

    def data_intact(self):
        return all((self.root / p).is_file() and (self.root / p).read_text() == p for p in DATA)


class SandboxTest(unittest.TestCase):
    def sandbox(self, **options):
        cache = ROOT / ".cache"
        cache.mkdir(exist_ok=True)
        directory = tempfile.TemporaryDirectory(prefix="ms-", dir=cache)
        self.addCleanup(directory.cleanup)
        return Sandbox(Path(directory.name), **options)

    def assertOk(self, result):
        self.assertEqual(result.returncode, 0, result.stderr)

    def assertFailed(self, result):
        self.assertNotEqual(result.returncode, 0, result.stderr)


STOPPING = [
    ("remove",),
    ("remove", "in-favour", "other", "1.0"),
    ("upgrade", "0.9.0"),
    ("failed-upgrade", "0.8.0", "0.9.0"),
    ("deconfigure", "in-favour", "other", "1.0", "removing", "conflict", "2.0"),
]


class PrermTests(SandboxTest):
    def test_every_stopping_argument_stops_core_before_executors_and_optional_units(self):
        for args in STOPPING:
            with self.subTest(args=args):
                box = self.sandbox()
                box.installed(running=[*ENABLED, READER, TARGETS])
                self.assertOk(box.run("prerm", *args))
                stops = [c.removeprefix("stop ") for c in box.mutating()]
                self.assertEqual(stops, [CORE, *EXECUTORS, MOUNTS, READER, READY, TARGETS])
                for name in [*ENABLED, *OPTIONAL]:
                    self.assertEqual(box.active(name), "inactive", name)
                # Effect units are stopped but never recorded for a restart.
                self.assertEqual(box.record(), [CORE, *EXECUTORS, READER])
                self.assertTrue(box.data_intact())

    def test_running_twice_is_idempotent(self):
        box = self.sandbox()
        box.installed(running=[*ENABLED, READER])
        self.assertOk(box.run("prerm", "upgrade", "0.9.0"))
        first = box.record()
        self.assertOk(box.run("prerm", "upgrade", "0.9.0"))
        self.assertEqual(box.record(), first)
        self.assertEqual(first, [CORE, *EXECUTORS, READER])

    def test_already_removed_units_are_tolerated(self):
        box = self.sandbox()
        # A second removal: dpkg already deleted every unit file.
        for name in [*ENABLED, *OPTIONAL]:
            box.unit(name, load="not-found")
        self.assertOk(box.run("prerm", "remove"))
        self.assertOk(box.run("prerm", "remove"))
        self.assertEqual(box.mutating(), [])
        self.assertIsNone(box.record())

    def test_never_enabled_or_never_started_units_are_tolerated(self):
        box = self.sandbox()
        box.installed(enabled=False)
        self.assertOk(box.run("prerm", "remove"))
        self.assertIsNone(box.record())

    def test_masked_core_unit_is_stopped_not_refused(self):
        box = self.sandbox()
        box.installed()
        box.unit(CORE, load="masked")
        self.assertOk(box.run("prerm", "remove"))
        self.assertIn(f"stop {CORE}", box.calls())

    def test_absent_core_unit_with_live_work_refuses(self):
        box = self.sandbox()
        box.installed()
        box.unit(CORE, load="not-found", active="active")
        result = box.run("prerm", "remove")
        self.assertFailed(result)
        self.assertIn("still has active or uncertain work", result.stderr)
        self.assertNotIn(f"stop {EXECUTORS[0]}", box.calls())

    def test_uncertain_core_load_state_refuses(self):
        box = self.sandbox()
        box.installed()
        box.unit(CORE, load="error")
        self.assertFailed(box.run("prerm", "upgrade", "0.9.0"))
        self.assertNotIn(f"stop {CORE}", box.calls())

    def test_without_a_running_manager_nothing_is_called(self):
        for args in STOPPING:
            with self.subTest(args=args):
                box = self.sandbox(systemd=False)
                box.installed(running=[*ENABLED, READER])
                self.assertOk(box.run("prerm", *args))
                self.assertEqual(box.calls(), [])
                self.assertIsNone(box.record())

    def test_a_failing_stop_aborts_and_keeps_what_it_must_restart(self):
        box = self.sandbox()
        box.installed(running=[*ENABLED, READER])
        box.fail(f"stop {CORE}")
        result = box.run("prerm", "upgrade", "0.9.0")
        self.assertFailed(result)
        self.assertIn("could not be stopped", result.stderr)
        self.assertNotIn(f"stop {EXECUTORS[0]}", box.calls())
        self.assertEqual(box.record(), [CORE])

    def test_a_failing_manager_query_aborts(self):
        box = self.sandbox()
        box.installed(running=ENABLED)
        box.fail(f"show {CORE} -p LoadState --value")
        self.assertFailed(box.run("prerm", "remove"))
        self.assertEqual(box.mutating(), [])

    def test_unknown_argument_fails_without_effect(self):
        box = self.sandbox()
        box.installed(running=ENABLED)
        result = box.run("prerm", "bogus")
        self.assertFailed(result)
        self.assertIn("unknown argument", result.stderr)
        self.assertEqual(box.calls(), [])

    def test_symlinked_record_directory_is_refused(self):
        box = self.sandbox()
        box.installed(running=ENABLED)
        elsewhere = box.root / "elsewhere"
        elsewhere.mkdir()
        (box.root / "run/limeos-package").symlink_to(elsewhere)
        self.assertFailed(box.run("prerm", "upgrade", "0.9.0"))
        self.assertEqual(list(elsewhere.iterdir()), [])


class PostrmTests(SandboxTest):
    def test_remove_and_purge_disable_reload_and_keep_every_data_path(self):
        for action in ("remove", "purge"):
            with self.subTest(action=action):
                box = self.sandbox()
                box.installed()
                (box.root / "run/limeos-package").mkdir(parents=True)
                (box.root / RECORD).write_text(f"{READER}\n")
                self.assertOk(box.run("postrm", action))
                for name in ENABLED:
                    self.assertFalse((box.root / WANTS / name).is_symlink(), name)
                self.assertFalse((box.root / "run/limeos-package").exists())
                self.assertEqual(box.calls(), ["daemon-reload"])
                self.assertTrue(box.data_intact())

    def test_remove_then_purge_and_twice_is_idempotent(self):
        box = self.sandbox()
        box.installed()
        for action in ("remove", "remove", "purge", "purge"):
            with self.subTest(action=action):
                self.assertOk(box.run("postrm", action))
        self.assertTrue(box.data_intact())

    def test_never_enabled_units_are_tolerated(self):
        box = self.sandbox()
        box.installed(enabled=False)
        self.assertOk(box.run("postrm", "remove"))

    def test_without_a_running_manager_links_are_still_removed(self):
        box = self.sandbox(systemd=False)
        box.installed()
        self.assertOk(box.run("postrm", "purge"))
        self.assertEqual(box.calls(), [])
        for name in ENABLED:
            self.assertFalse((box.root / WANTS / name).is_symlink(), name)

    def test_purge_after_systemd_itself_was_removed(self):
        box = self.sandbox(systemctl=False)
        box.installed()
        self.assertOk(box.run("postrm", "purge"))
        self.assertTrue(box.data_intact())

    def test_a_failing_reload_does_not_fail_the_removal(self):
        box = self.sandbox()
        box.installed()
        box.fail("daemon-reload")
        result = box.run("postrm", "remove")
        self.assertOk(result)
        self.assertIn("daemon-reload", result.stderr)

    def test_only_symlinks_are_removed_from_the_wants_directory(self):
        box = self.sandbox()
        box.installed(enabled=False)
        wants = box.root / WANTS
        wants.mkdir(parents=True)
        (wants / CORE).write_text("operator file\n")
        (wants / "other.service").symlink_to("/lib/systemd/system/other.service")
        self.assertOk(box.run("postrm", "purge"))
        self.assertEqual((wants / CORE).read_text(), "operator file\n")
        self.assertTrue((wants / "other.service").is_symlink())

    def test_non_removal_arguments_change_nothing_and_keep_the_restart_record(self):
        for args in [
            ("upgrade", "1.0.0"),
            ("failed-upgrade", "0.9.0", "1.0.0"),
            ("abort-install",),
            ("abort-install", "0.9.0"),
            ("abort-upgrade", "0.9.0", "1.0.0"),
            ("disappear", "other", "1.0"),
        ]:
            with self.subTest(args=args):
                box = self.sandbox()
                box.installed()
                (box.root / "run/limeos-package").mkdir(parents=True)
                (box.root / RECORD).write_text(f"{READER}\n")
                self.assertOk(box.run("postrm", *args))
                self.assertEqual(box.calls(), [])
                self.assertEqual(box.record(), [READER])
                for name in ENABLED:
                    self.assertTrue((box.root / WANTS / name).is_symlink(), name)

    def test_unknown_argument_fails(self):
        box = self.sandbox()
        box.installed()
        result = box.run("postrm", "bogus")
        self.assertFailed(result)
        self.assertIn("unknown argument", result.stderr)
        self.assertTrue((box.root / WANTS / CORE).is_symlink())


def restore_function():
    script = (DEBIAN / "postinst").read_text()
    match = re.search(r"^fct_restore_units\(\) \{\n.*?^\}\n", script, re.M | re.S)
    assert match, "postinst defines fct_restore_units"
    directory = re.search(r"^RESTART_DIRECTORY=.*\n", script, re.M)
    assert directory, "postinst names its restart record"
    return directory[0] + match[0]


class PostinstTests(SandboxTest):
    ABORTS = [
        ("abort-upgrade", "1.0.0"),
        ("abort-remove",),
        ("abort-remove", "in-favour", "other", "1.0"),
        ("abort-deconfigure", "in-favour", "other", "1.0", "removing", "conflict", "2.0"),
    ]

    def test_every_abort_restarts_exactly_what_prerm_stopped(self):
        for args in self.ABORTS:
            with self.subTest(args=args):
                box = self.sandbox()
                box.installed(running=[*ENABLED, READER, TARGETS])
                self.assertOk(box.run("prerm", "upgrade", "1.0.0"))
                box.clear()
                self.assertOk(box.run("postinst", *args))
                # Reverse of the stop order: the reader and executors before core.
                self.assertEqual(
                    box.mutating(),
                    ["daemon-reload", f"start {READER}", f"start {EXECUTORS[1]}",
                     f"start {EXECUTORS[0]}", f"start {CORE}"],
                )
                self.assertEqual(box.active(TARGETS), "inactive")
                self.assertIsNone(box.record())
                self.assertFalse((box.root / "run/limeos-package").exists())
                box.clear()
                # A second abort has nothing left to restart.
                self.assertOk(box.run("postinst", *args))
                self.assertEqual(box.mutating(), ["daemon-reload"])

    def test_abort_without_a_record_starts_nothing(self):
        box = self.sandbox()
        box.installed()
        self.assertOk(box.run("postinst", "abort-upgrade", "1.0.0"))
        self.assertEqual(box.mutating(), ["daemon-reload"])

    def test_abort_without_a_running_manager_calls_nothing(self):
        box = self.sandbox(systemd=False)
        box.installed()
        (box.root / "run/limeos-package").mkdir(parents=True)
        (box.root / RECORD).write_text(f"{CORE}\n")
        self.assertOk(box.run("postinst", "abort-remove"))
        self.assertEqual(box.calls(), [])

    def test_abort_with_a_failing_start_starts_the_rest_and_keeps_the_record(self):
        box = self.sandbox()
        box.installed(running=[*ENABLED, READER])
        self.assertOk(box.run("prerm", "remove"))
        box.fail(f"start {READER}")
        box.clear()
        result = box.run("postinst", "abort-remove")
        self.assertFailed(result)
        self.assertIn(READER, result.stderr)
        self.assertEqual(box.active(CORE), "active")
        self.assertEqual(box.record(), [CORE, *EXECUTORS, READER])
        (box.state / "fail").unlink()
        self.assertOk(box.run("postinst", "abort-remove"))
        self.assertIsNone(box.record())

    def test_unknown_argument_fails_without_effect(self):
        box = self.sandbox()
        box.installed()
        result = box.run("postinst", "bogus")
        self.assertFailed(result)
        self.assertIn("unknown argument", result.stderr)
        self.assertEqual(box.calls(), [])

    def test_upgrade_restarts_the_running_reader_but_leaves_effect_units_dormant(self):
        box = self.sandbox()
        box.installed(running=[*ENABLED, READER, TARGETS, MOUNTS])
        self.assertOk(box.run("prerm", "upgrade", "1.0.0"))
        self.assertOk(box.run("postrm", "upgrade", "1.0.0"))
        self.assertEqual(box.record(), [CORE, *EXECUTORS, READER])
        box.clear()
        # configure restarts the core units itself, then restores the rest.
        program = restore_function() + f"fct_restore_units {' '.join(ENABLED)}\n"
        self.assertOk(box.run("postinst", source=program))
        self.assertEqual(box.mutating(), [f"start {READER}"])
        self.assertEqual(box.active(TARGETS), "inactive")
        self.assertEqual(box.active(MOUNTS), "inactive")
        self.assertIsNone(box.record())

    def test_upgrade_without_the_reader_running_leaves_it_stopped(self):
        box = self.sandbox()
        box.installed(running=ENABLED)
        self.assertOk(box.run("prerm", "upgrade", "1.0.0"))
        box.clear()
        program = restore_function() + f"fct_restore_units {' '.join(ENABLED)}\n"
        self.assertOk(box.run("postinst", source=program))
        self.assertEqual(box.mutating(), [])
        self.assertEqual(box.active(READER), "inactive")

    def test_restore_ignores_entries_that_are_not_package_units(self):
        box = self.sandbox()
        box.installed()
        (box.root / "run/limeos-package").mkdir(parents=True)
        (box.root / RECORD).write_text(f"sshd.service\n../x.service\n{READER}\n")
        program = restore_function() + "fct_restore_units\n"
        result = box.run("postinst", source=program)
        self.assertOk(result)
        self.assertEqual(box.mutating(), [f"start {READER}"])
        self.assertIn("sshd.service", result.stderr)

    def test_configure_restores_after_restarting_core_and_before_readiness(self):
        script = (DEBIAN / "postinst").read_text()
        configure = script[script.index("fct_execute_this() {"):]
        restart = configure.index("systemctl restart")
        restore = configure.index("fct_restore_units ")
        ready = configure.index("for (( attempt=0")
        self.assertLess(restart, restore)
        self.assertLess(restore, ready)
        self.assertIn(f"fct_restore_units {' '.join(ENABLED)}", configure)


class PackageConsistencyTests(unittest.TestCase):
    def install_targets(self, name):
        text = (SYSTEMD / name).read_text()
        if "[Install]" not in text:
            return None
        section = text.split("[Install]", 1)[1]
        return [line.strip() for line in section.splitlines() if "=" in line]

    def test_postrm_link_sweep_matches_every_enabled_unit(self):
        """postrm removes links without the unit files; they must say where the links are."""
        enabled = sorted(p.name for p in SYSTEMD.glob("*.service") if self.install_targets(p.name))
        self.assertEqual(enabled, sorted(ENABLED))
        for name in enabled:
            self.assertEqual(self.install_targets(name), ["WantedBy=multi-user.target"], name)
        postinst = (DEBIAN / "postinst").read_text()
        self.assertIn(f"systemctl enable {' '.join(ENABLED)}", postinst)

    def test_shadow_profile_scopes_the_record_and_unit_names(self):
        build = load_build()
        for script in ("prerm", "postrm", "postinst"):
            with self.subTest(script=script):
                shadow = build.profile_text((DEBIAN / script).read_text(), True)
                self.assertNotIn("/run/limeos-package", shadow)
                for name in [*ENABLED, *OPTIONAL]:
                    self.assertNotIn(f" {name}", shadow)
                self.assertIn("/run/limeos-shadow-package", shadow)

    def test_scripts_use_strict_mode(self):
        for script in ("prerm", "postrm", "postinst"):
            with self.subTest(script=script):
                self.assertIn("\nset -Eeuo pipefail\n", (DEBIAN / script).read_text())

    @unittest.skipUnless(shutil.which("shellcheck"), "shellcheck is not installed")
    def test_scripts_pass_shellcheck(self):
        scripts = [str(DEBIAN / name) for name in ("postinst", "prerm", "postrm")]
        result = subprocess.run(["shellcheck", *scripts], capture_output=True, text=True, check=False)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)


if __name__ == "__main__":
    unittest.main()
