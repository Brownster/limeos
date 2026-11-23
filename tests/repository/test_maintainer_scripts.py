"""Stub-driven tests for packaging/debian/{prerm,postrm,postinst} (PR-24).

No real systemctl, account tool or limeosctl is ever run and no host path is
touched: every script runs with PATH set to a sandbox holding stubs and a few
named coreutils, and with DPKG_ROOT (which dpkg itself sets for maintainer
scripts) pointing at a temp root, under which the scripts find
run/systemd/system, the cgroup tree, the enablement links and their restart
record. postinst's configure step also uses fixed /etc/limeos, /var/lib/limeos
and /usr/lib/limeos paths; the tests run the complete script with only those
path prefixes moved under the temp root.

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
ALL = [*ENABLED, *OPTIONAL]
RECORD_DIR = "run/limeos-package"
RECORD = f"{RECORD_DIR}/units"
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
TOOLS = ["mkdir", "mktemp", "mv", "rm", "stat", "cut", "chmod"]

SYSTEMCTL = r"""#!/bin/bash
# A model of systemctl over $STUB_STATE/units/UNIT.{load,active,cgroup,unitfile,survive}.
set -u
state="${STUB_STATE}"
call="$*"
printf '%s\n' "${call}" >>"${state}/calls.log"
if [[ -f "${state}/fail" ]]; then
    while IFS= read -r line; do
        if [[ "${line}" == "${call}" ]]; then printf 'stub: injected failure: %s\n' "${call}" >&2; exit 1; fi
    done <"${state}/fail"
fi
# Calls in fail-after take effect, then report failure (a stop job that
# errors although the unit did stop).
late=0
if [[ -f "${state}/fail-after" ]]; then
    while IFS= read -r line; do [[ "${line}" != "${call}" ]] || late=1; done <"${state}/fail-after"
fi
trap '(( late == 0 )) || { printf "stub: late failure: %s\n" "${call}" >&2; exit 1; }' EXIT
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
            UnitFileState) get "${unit}" unitfile static ;;
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
            # The stop job completes, but processes may survive it.
            if [[ -f "${state}/units/${unit}.survive" ]]; then
                put "${unit}" active "$(get "${unit}" survive failed)"
            else
                put "${unit}" active inactive
                command -p rm -f "${state}/units/${unit}.cgroup"
            fi
        done ;;
    start|restart)
        for unit in "$@"; do
            case "$(get "${unit}" load not-found)" in
                loaded) put "${unit}" active active ;;
                masked) printf 'Unit %s is masked.\n' "${unit}" >&2; exit 1 ;;
                *) printf 'Unit %s not found.\n' "${unit}" >&2; exit 5 ;;
            esac
        done ;;
    is-active)
        for unit in "$@"; do
            [[ "${unit}" == --quiet ]] && continue
            [[ "$(get "${unit}" active inactive)" == active ]] || exit 3
        done ;;
    enable) for unit in "$@"; do put "${unit}" unitfile enabled; done ;;
    disable|daemon-reload) ;;
    *) exit 97 ;;
esac
"""

# Account and file tools postinst's configure step calls; all record the call.
TOOL_STUB = r"""#!/bin/bash
set -u
tool="${0##*/}"
call="${tool} $*"
printf '%s\n' "${call}" >>"${STUB_STATE}/calls.log"
if [[ -f "${STUB_STATE}/fail" ]]; then
    while IFS= read -r line; do
        [[ "${line}" != "${call}" ]] || { printf 'stub: injected failure: %s\n' "${call}" >&2; exit 1; }
    done <"${STUB_STATE}/fail"
fi
case "${tool}" in
    getent)
        if [[ "${1}" == passwd ]]; then printf '%s:x:150:150::/nonexistent:/usr/sbin/nologin\n' "${2}"; else printf '%s:x:150:\n' "${2}"; fi ;;
    id)
        if [[ "${1}" == -u ]]; then printf '150\n'; else printf '%s\n' "${2}"; fi ;;
    install)
        mode='' directory=''
        while (( $# )); do case "${1}" in -m) mode="${2}"; shift ;; -o|-g) shift ;; -d) ;; *) directory="${1}" ;; esac; shift; done
        mkdir -p -- "${directory}" && chmod "${mode}" -- "${directory}" ;;
    *) ;;
esac
"""
TOOL_STUBS = ["getent", "id", "install", "chown", "sync", "sleep", "adduser", "addgroup"]


def load_build():
    spec = importlib.util.spec_from_file_location("package_build", ROOT / "packaging/build.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class Sandbox:
    """A temp root, stub tools and a PATH holding nothing else."""

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
            self.executable(self.bin / "systemctl", SYSTEMCTL)
        for tool in TOOL_STUBS:
            self.executable(self.bin / tool, TOOL_STUB)
        self.executable(self.root / "usr/lib/limeos/limeosctl", TOOL_STUB)
        (self.root / "run").mkdir()
        if systemd:
            (self.root / "run/systemd/system").mkdir(parents=True)
        cgroup = self.root / "sys/fs/cgroup"
        cgroup.mkdir(parents=True)
        (cgroup / "cgroup.controllers").write_text("cpu memory pids\n")
        for path in DATA:
            target = self.root / path
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text(path)

    @staticmethod
    def executable(path, text):
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)
        path.chmod(0o755)

    def set(self, name, **values):
        for key, value in values.items():
            (self.state / f"units/{name}.{key}").write_text(f"{value}\n")

    def unit(self, name, load="loaded", active="inactive"):
        self.set(name, load=load, active=active)

    def active(self, name):
        path = self.state / f"units/{name}.active"
        return path.read_text().strip() if path.exists() else "inactive"

    def states(self):
        return {name: self.active(name) for name in ALL}

    def installed(self, running=(), enabled=True):
        for name in ALL:
            self.unit(name, active="active" if name in running else "inactive")
        if enabled:
            wants = self.root / WANTS
            wants.mkdir(parents=True, exist_ok=True)
            for name in ENABLED:
                self.set(name, unitfile="enabled")
                (wants / name).symlink_to(f"/lib/systemd/system/{name}")

    def surviving(self, name, state="failed", populated=1, events=None, procs="4242\n"):
        """The next stop of NAME completes but leaves its cgroup behind."""
        cgroup = f"/system.slice/{name}"
        self.set(name, survive=state, cgroup=cgroup)
        directory = self.root / f"sys/fs/cgroup{cgroup}"
        directory.mkdir(parents=True, exist_ok=True)
        if events is not False:
            (directory / "cgroup.events").write_text(
                events if events is not None else f"populated {populated}\nfrozen 0\n"
            )
        (directory / "cgroup.procs").write_text(procs)

    def fail(self, *calls):
        (self.state / "fail").write_text("".join(f"{call}\n" for call in calls))

    def heal(self):
        (self.state / "fail").unlink(missing_ok=True)
        (self.state / "fail-after").unlink(missing_ok=True)

    def calls(self):
        return (self.state / "calls.log").read_text().splitlines()

    def manager(self):
        """Mutating systemctl calls only."""
        return [c for c in self.calls() if not c.startswith(("show ", "is-active ")) and c.split()[0] in
                {"stop", "start", "restart", "enable", "disable", "daemon-reload"}]

    def clear(self):
        (self.state / "calls.log").write_text("")

    def write_record(self, *units, header="upgrade 1.0", mode=0o600):
        """UNIT (recorded enabled) or a full "UNIT STATE" line."""
        directory = self.root / RECORD_DIR
        directory.mkdir(mode=0o700, exist_ok=True)
        lines = [] if header is None else [header]
        lines += [u if " " in u else f"{u} enabled" for u in units]
        (directory / "units").write_text("".join(f"{line}\n" for line in lines))
        (directory / "units").chmod(mode)

    def record_lines(self):
        path = self.root / RECORD
        return path.read_text().splitlines() if path.exists() else None

    def record(self):
        """The recorded units, without the header and states."""
        lines = self.record_lines()
        return None if lines is None else [line.split()[0] for line in lines[1:]]

    def fail_after(self, *calls):
        (self.state / "fail-after").write_text("".join(f"{call}\n" for call in calls))

    def record_exists(self):
        return (self.root / RECORD_DIR).exists() or (self.root / RECORD_DIR).is_symlink()

    def program(self, script):
        text = (DEBIAN / script).read_text()
        if script == "postinst":
            # configure's fixed paths, moved under the temp root.
            for prefix in ("/usr/lib/limeos/", "/etc/limeos", "/var/lib/limeos"):
                text = text.replace(prefix, f"{self.root}{prefix}")
        return text

    def run(self, script, *args, source=None):
        program = source if source is not None else self.program(script)
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
    maxDiff = None

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

    def assertStates(self, box, active):
        expected = {name: "active" if name in active else "inactive" for name in ALL}
        self.assertEqual(box.states(), expected)


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
                stops = [c.removeprefix("stop ") for c in box.manager()]
                self.assertEqual(stops, [CORE, *EXECUTORS, MOUNTS, READER, READY, TARGETS])
                self.assertStates(box, [])
                # Effect units are stopped but never recorded for a restart.
                self.assertEqual(box.record(), [CORE, *EXECUTORS, READER])
                header = {"remove": "remove", "deconfigure": "deconfigure", "upgrade": "upgrade 0.9.0",
                          "failed-upgrade": "upgrade 0.9.0"}[args[0]]
                self.assertEqual(box.record_lines()[0], header)
                self.assertEqual(box.record_lines()[1:], [f"{u} enabled" for u in [CORE, *EXECUTORS]]
                                 + [f"{READER} enabled"])
                self.assertEqual((box.root / RECORD_DIR).stat().st_mode & 0o777, 0o700)
                self.assertEqual((box.root / RECORD).stat().st_mode & 0o777, 0o600)
                self.assertTrue(box.data_intact())

    def test_running_twice_succeeds_and_the_second_run_replaces_the_record(self):
        box = self.sandbox()
        box.installed(running=[*ENABLED, READER])
        self.assertOk(box.run("prerm", "upgrade", "0.9.0"))
        self.assertEqual(box.record(), [CORE, *EXECUTORS, READER])
        self.assertOk(box.run("prerm", "upgrade", "0.9.0"))
        self.assertEqual(box.record(), [])
        self.assertStates(box, [])

    def test_a_stale_record_is_replaced_not_extended(self):
        for action in (("remove",), ("upgrade", "1.0"), ("deconfigure", "in-favour", "x", "1")):
            with self.subTest(action=action):
                box = self.sandbox()
                box.installed(running=ENABLED)
                # Left by an interrupted run; the administrator stopped the reader since.
                box.write_record(CORE, READER)
                self.assertOk(box.run("prerm", *action))
                self.assertEqual(box.record(), ENABLED)

    def test_failed_upgrade_extends_the_record_of_the_failed_upgrade(self):
        box = self.sandbox()
        box.installed(running=[EXECUTORS[1], READER])
        box.write_record(CORE, f"{EXECUTORS[0]} disabled", EXECUTORS[1], TARGETS, "sshd.service",
                         f"{CORE} bogus", header="upgrade 1.0.0")
        self.assertOk(box.run("prerm", "failed-upgrade", "0.9.0", "1.0.0"))
        # Entries the package never restarts, or with an unknown state, are dropped.
        self.assertEqual(box.record_lines(), ["upgrade 1.0.0", f"{CORE} enabled", f"{EXECUTORS[0]} disabled",
                                              f"{EXECUTORS[1]} enabled", f"{READER} enabled"])

    def test_failed_upgrade_replaces_a_record_this_upgrade_did_not_write(self):
        for header in ("remove", "deconfigure", "upgrade 9.9", None, f"{CORE} enabled"):
            with self.subTest(header=header):
                box = self.sandbox()
                box.installed(running=[READER])
                box.write_record(CORE, EXECUTORS[0], header=header)
                self.assertOk(box.run("prerm", "failed-upgrade", "0.9.0", "1.0.0"))
                self.assertEqual(box.record_lines(), ["upgrade 1.0.0", f"{READER} enabled"])

    def test_enablement_is_recorded_with_each_unit(self):
        box = self.sandbox()
        box.installed(running=[*ENABLED, READER])
        box.set(CORE, unitfile="disabled")
        self.assertOk(box.run("prerm", "remove"))
        self.assertEqual(box.record_lines(), ["remove", f"{CORE} disabled", f"{EXECUTORS[0]} enabled",
                                              f"{EXECUTORS[1]} enabled", f"{READER} enabled"])

    def test_already_removed_units_are_tolerated(self):
        box = self.sandbox()
        # A second removal: dpkg already deleted every unit file.
        for name in ALL:
            box.unit(name, load="not-found")
        self.assertOk(box.run("prerm", "remove"))
        self.assertOk(box.run("prerm", "remove"))
        self.assertEqual(box.manager(), [])
        self.assertEqual(box.record(), [])

    def test_never_enabled_or_never_started_units_are_tolerated(self):
        box = self.sandbox()
        box.installed(enabled=False)
        self.assertOk(box.run("prerm", "remove"))
        self.assertEqual(box.record(), [])

    def test_masked_core_unit_is_stopped_not_refused(self):
        box = self.sandbox()
        box.installed()
        box.unit(CORE, load="masked")
        self.assertOk(box.run("prerm", "remove"))
        self.assertIn(f"stop {CORE}", box.calls())

    def test_masked_dormant_optional_unit_is_skipped(self):
        for name in OPTIONAL:
            with self.subTest(unit=name):
                box = self.sandbox()
                box.installed()
                box.unit(name, load="masked")
                self.assertOk(box.run("prerm", "remove"))
                self.assertNotIn(f"stop {name}", box.calls())

    def test_masked_optional_unit_with_work_refuses(self):
        box = self.sandbox()
        box.installed()
        box.unit(READER, load="masked", active="active")
        result = box.run("prerm", "remove")
        self.assertFailed(result)
        self.assertIn("still has active or uncertain work", result.stderr)
        box.unit(READER, load="masked")
        (box.root / f"sys/fs/cgroup/system.slice/{READER}").mkdir(parents=True)
        self.assertFailed(box.run("prerm", "remove"))

    def test_masked_dormant_optional_unit_with_an_empty_retained_cgroup_is_skipped(self):
        for control_group in ("", f"/system.slice/{READER}"):
            with self.subTest(control_group=control_group):
                box = self.sandbox()
                box.installed()
                box.unit(READER, load="masked")
                box.set(READER, cgroup=control_group)
                directory = box.root / f"sys/fs/cgroup/system.slice/{READER}"
                directory.mkdir(parents=True)
                (directory / "cgroup.events").write_text("populated 0\nfrozen 0\n")
                self.assertOk(box.run("prerm", "remove"))
                self.assertNotIn(f"stop {READER}", box.calls())
                (directory / "cgroup.events").write_text("populated 1\nfrozen 0\n")
                self.assertFailed(box.run("prerm", "remove"))
                (directory / "cgroup.events").unlink()
                self.assertFailed(box.run("prerm", "remove"))

    def test_without_the_unified_hierarchy_a_missing_cgroup_is_uncertain(self):
        box = self.sandbox()
        box.installed(running=ENABLED)
        box.surviving(CORE)
        shutil.rmtree(box.root / f"sys/fs/cgroup/system.slice/{CORE}")
        (box.root / "sys/fs/cgroup/cgroup.controllers").unlink()
        result = box.run("prerm", "remove")
        self.assertFailed(result)
        self.assertIn(f"Cannot tell whether {CORE} stopped", result.stderr)
        # A masked unit's retained cgroup cannot be judged either.
        box = self.sandbox()
        box.installed()
        box.unit(READER, load="masked")
        box.set(READER, cgroup=f"/system.slice/{READER}")
        (box.root / "sys/fs/cgroup/cgroup.controllers").unlink()
        self.assertFailed(box.run("prerm", "remove"))

    def test_a_stop_error_after_the_unit_did_stop_continues_with_a_warning(self):
        box = self.sandbox()
        box.installed(running=[*ENABLED, READER])
        box.fail_after(f"stop {CORE}", f"stop {READER}")
        result = box.run("prerm", "upgrade", "1.0")
        self.assertOk(result)
        self.assertIn(f"systemctl stop {CORE} failed, but the unit has stopped", result.stderr)
        self.assertIn(f"systemctl stop {READER} failed", result.stderr)
        self.assertStates(box, [])

    def test_a_stop_error_with_work_left_or_no_answer_fails(self):
        box = self.sandbox()
        box.installed(running=ENABLED)
        box.surviving(CORE)
        box.fail_after(f"stop {CORE}")
        result = box.run("prerm", "upgrade", "1.0")
        self.assertFailed(result)
        self.assertIn(f"Processes of {CORE} survived", result.stderr)
        box = self.sandbox()
        box.installed(running=ENABLED)
        box.fail_after(f"stop {CORE}")
        box.fail(f"show {CORE} -p ControlGroup --value")
        result = box.run("prerm", "upgrade", "1.0")
        self.assertFailed(result)
        self.assertIn("Cannot tell", result.stderr)

    def test_processes_surviving_a_completed_stop_abort_and_are_named(self):
        for name in [CORE, EXECUTORS[0], READER, READY]:
            with self.subTest(unit=name):
                box = self.sandbox()
                box.installed(running=[*ENABLED, READER])
                box.surviving(name)
                result = box.run("prerm", "upgrade", "1.0")
                self.assertFailed(result)
                self.assertIn(f"Processes of {name} survived its stop", result.stderr)
                self.assertIn("4242", result.stderr)
                order = [CORE, *EXECUTORS, MOUNTS, READER, READY, TARGETS]
                stops = [c.removeprefix("stop ") for c in box.manager()]
                self.assertEqual(stops, order[: order.index(name) + 1])

    def test_a_unit_still_deactivating_after_its_stop_aborts(self):
        box = self.sandbox()
        box.installed(running=ENABLED)
        box.surviving(CORE, state="deactivating", populated=0)
        self.assertFailed(box.run("prerm", "remove"))

    def test_failed_unit_with_an_empty_or_removed_cgroup_is_stopped(self):
        box = self.sandbox()
        box.installed(running=ENABLED)
        box.surviving(CORE, populated=0)
        box.surviving(EXECUTORS[0])
        shutil.rmtree(box.root / f"sys/fs/cgroup/system.slice/{EXECUTORS[0]}")
        self.assertOk(box.run("prerm", "remove"))

    def test_uncertain_answers_after_a_stop_abort(self):
        cases = {
            "no events file": dict(events=False),
            "no populated line": dict(events="frozen 0\n"),
            "garbled events": dict(events="populated maybe\n"),
        }
        for label, options in cases.items():
            with self.subTest(case=label):
                box = self.sandbox()
                box.installed(running=ENABLED)
                box.surviving(CORE, **options)
                result = box.run("prerm", "upgrade", "1.0")
                self.assertFailed(result)
                self.assertIn(f"Cannot tell whether {CORE} stopped", result.stderr)
        with self.subTest(case="foreign cgroup path"):
            box = self.sandbox()
            box.installed(running=ENABLED)
            box.surviving(CORE, populated=0)
            box.set(CORE, cgroup="/user.slice/other")
            self.assertFailed(box.run("prerm", "upgrade", "1.0"))
        with self.subTest(case="empty active state"):
            box = self.sandbox()
            box.installed(running=ENABLED)
            box.surviving(CORE, state="", populated=0)
            result = box.run("prerm", "upgrade", "1.0")
            self.assertFailed(result)
            self.assertIn("Cannot tell", result.stderr)
        with self.subTest(case="manager query fails after the stop"):
            box = self.sandbox()
            box.installed(running=ENABLED)
            box.fail(f"show {CORE} -p ControlGroup --value")
            self.assertFailed(box.run("prerm", "upgrade", "1.0"))

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
                self.assertFalse(box.record_exists())

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
        self.assertEqual(box.manager(), [])

    def test_unknown_argument_fails_without_effect(self):
        box = self.sandbox()
        box.installed(running=ENABLED)
        result = box.run("prerm", "bogus")
        self.assertFailed(result)
        self.assertIn("unknown argument", result.stderr)
        self.assertEqual(box.calls(), [])

    def test_an_unsafe_record_is_reported_and_never_stops_the_change(self):
        def symlinked(box):
            elsewhere = box.root / "elsewhere"
            elsewhere.mkdir()
            (box.root / RECORD_DIR).symlink_to(elsewhere)
            return elsewhere

        def open_directory(box):
            (box.root / RECORD_DIR).mkdir()
            (box.root / RECORD_DIR).chmod(0o777)
            return box.root / RECORD_DIR

        def open_file(box):
            box.write_record(READER, mode=0o666)
            return box.root / RECORD_DIR

        def linked_file(box):
            (box.root / RECORD_DIR).mkdir(mode=0o700)
            (box.root / "target").write_text(f"{READER}\n")
            (box.root / RECORD).symlink_to(box.root / "target")
            return box.root / RECORD_DIR

        for make in (symlinked, open_directory, open_file, linked_file):
            for args in (("upgrade", "1.0"), ("failed-upgrade", "0.9", "1.0")):
                with self.subTest(case=make.__name__, args=args):
                    box = self.sandbox()
                    box.installed(running=[*ENABLED, READER])
                    directory = make(box)
                    before = sorted(p.name for p in directory.iterdir())
                    result = box.run("prerm", *args)
                    self.assertOk(result)
                    self.assertIn("unsafe restart record", result.stderr)
                    self.assertStates(box, [])
                    self.assertEqual(sorted(p.name for p in directory.iterdir()), before)


class PostrmTests(SandboxTest):
    def test_remove_and_purge_disable_reload_and_keep_every_data_path(self):
        for action in ("remove", "purge"):
            with self.subTest(action=action):
                box = self.sandbox()
                box.installed()
                box.write_record(READER)
                self.assertOk(box.run("postrm", action))
                for name in ENABLED:
                    self.assertFalse((box.root / WANTS / name).is_symlink(), name)
                self.assertFalse(box.record_exists())
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
                box.write_record(READER)
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


ABORTS = [
    ("abort-upgrade", "1.0.0"),
    ("abort-remove",),
    ("abort-remove", "in-favour", "other", "1.0"),
    ("abort-deconfigure", "in-favour", "other", "1.0", "removing", "conflict", "2.0"),
]
CONFIGURE_TAIL = ["daemon-reload", f"enable {' '.join(ENABLED)}",
                  f"restart {EXECUTORS[1]} {EXECUTORS[0]} {CORE}"]


class PostinstTests(SandboxTest):
    def test_configure_on_a_fresh_install_starts_core_and_nothing_optional(self):
        box = self.sandbox()
        box.installed(enabled=False)
        result = box.run("postinst", "configure")
        self.assertOk(result)
        self.assertEqual(box.manager(), CONFIGURE_TAIL)
        self.assertStates(box, ENABLED)
        self.assertIn(f"limeosctl check-config {box.root}/etc/limeos/core.json", box.calls())
        self.assertEqual((box.root / "etc/limeos/system-policy/host.json").stat().st_mode & 0o777, 0o640)
        self.assertFalse(box.record_exists())

    def test_configure_without_a_running_manager_ends_the_transaction(self):
        box = self.sandbox(systemd=False)
        box.installed()
        box.write_record(*ENABLED, READER)
        self.assertOk(box.run("postinst", "configure"))
        self.assertEqual(box.manager(), [])
        self.assertFalse(box.record_exists())

    def test_configure_skips_a_recorded_unit_masked_or_disabled_since(self):
        for change in (dict(load="masked"), dict(unitfile="masked"), dict(unitfile="disabled")):
            with self.subTest(change=change):
                box = self.sandbox()
                box.installed(running=[*ENABLED, READER])
                self.assertOk(box.run("prerm", "upgrade", "1.0"))
                box.set(READER, **change)
                box.clear()
                result = box.run("postinst", "configure")
                self.assertOk(result)
                self.assertIn(f"not starting {READER}", result.stderr)
                self.assertEqual(box.manager(), CONFIGURE_TAIL)
                self.assertFalse(box.record_exists())

    def test_configure_names_a_reader_that_does_not_start_and_ends_the_transaction(self):
        box = self.sandbox()
        box.installed(running=[*ENABLED, READER])
        self.assertOk(box.run("prerm", "upgrade", "1.0"))
        box.fail(f"start {READER}")
        result = box.run("postinst", "configure")
        self.assertFailed(result)
        self.assertIn(f"start it with: systemctl start {READER}", result.stderr)
        self.assertStates(box, ENABLED)
        self.assertFalse(box.record_exists())
        # dpkg --configure again finishes; restarting the reader is now the operator's.
        box.heal()
        box.clear()
        self.assertOk(box.run("postinst", "configure"))
        self.assertEqual(box.manager(), CONFIGURE_TAIL)

    def check_ceiling(self, box, target, role=None):
        policy = box.root / f"etc/limeos/system-policy/{target}.json"
        return policy, f"limeosctl check-ceiling {policy} 150 {role or target}"

    def test_a_refused_ceiling_warns_once_keeps_the_file_and_starts_everything_else(self):
        """Configure must not fail: prerm stopped every unit, and a failed
        configure would leave core down and apt blocked for a file the
        executor refuses anyway."""
        for target, unit in (("container", EXECUTORS[0]), ("host", EXECUTORS[1])):
            with self.subTest(target=target):
                box = self.sandbox()
                box.installed(running=[*ENABLED, READER])
                self.assertOk(box.run("prerm", "upgrade", "1.0"))
                policy, check = self.check_ceiling(box, target)
                before = policy.read_bytes(), policy.stat().st_mode
                box.fail(check)
                box.clear()
                result = box.run("postinst", "configure")
                self.assertOk(result)
                warnings = [line for line in result.stderr.splitlines() if "LimeOS warning" in line]
                self.assertEqual(len(warnings), 1, result.stderr)
                for part in (
                    str(policy),
                    f"refused for the {target} executor role",
                    f"stub: injected failure: {check}",
                    f"{unit} is not (re)started",
                    f"check-ceiling {policy} 150 {target}",
                    f"then run: systemctl restart {unit}",
                ):
                    self.assertIn(part, warnings[0])
                self.assertEqual((policy.read_bytes(), policy.stat().st_mode), before)
                others = [u for u in EXECUTORS[::-1] if u != unit]
                self.assertEqual(
                    box.manager(),
                    ["daemon-reload", f"enable {' '.join(ENABLED)}",
                     f"restart {' '.join(others)} {CORE}", f"start {READER}"],
                )
                self.assertStates(box, [CORE, *others, READER])
                self.assertNotIn(f"chown root:limeos-host-access {policy}", box.calls())
                self.assertFalse(box.record_exists())

    def test_a_refused_reader_ceiling_keeps_the_reader_stopped(self):
        box = self.sandbox()
        box.installed(running=[*ENABLED, READER])
        self.assertOk(box.run("prerm", "upgrade", "1.0"))
        _, check = self.check_ceiling(box, "storage")
        box.fail(check)
        box.clear()
        result = box.run("postinst", "configure")
        self.assertOk(result)
        self.assertIn(f"{READER} is not (re)started", result.stderr)
        self.assertEqual(box.manager(), CONFIGURE_TAIL)
        self.assertStates(box, ENABLED)

    def test_the_shadow_postinst_checks_its_container_ceiling_as_read_only(self):
        script = load_build().maintainer_script("postinst", True)
        self.assertIn('role="${target}"; [[ "${target}" != container ]] || role=container-read-only', script)
        self.assertIn('check-ceiling "/etc/limeos-shadow/system-policy/${target}.json" "${core_uid}" "${role}"', script)
        standard = load_build().maintainer_script("postinst", False)
        self.assertNotIn("container-read-only", standard)

    def test_an_unsafe_record_is_ignored_with_a_warning(self):
        def symlinked(box):
            elsewhere = box.root / "elsewhere"
            elsewhere.mkdir(mode=0o700)
            (elsewhere / "units").write_text(f"{READER}\n")
            (elsewhere / "units").chmod(0o600)
            (box.root / RECORD_DIR).symlink_to(elsewhere)

        def open_directory(box):
            box.write_record(READER)
            (box.root / RECORD_DIR).chmod(0o777)

        def group_writable(box):
            box.write_record(READER, mode=0o620)

        def linked_file(box):
            (box.root / RECORD_DIR).mkdir(mode=0o700)
            (box.root / "target").write_text(f"{READER}\n")
            (box.root / RECORD).symlink_to(box.root / "target")

        def not_a_file(box):
            (box.root / RECORD).mkdir(parents=True)

        for make in (symlinked, open_directory, group_writable, linked_file, not_a_file):
            for args in (("configure",), ("abort-upgrade", "1.0")):
                with self.subTest(case=make.__name__, args=args):
                    box = self.sandbox()
                    box.installed()
                    make(box)
                    result = box.run("postinst", *args)
                    self.assertOk(result)
                    self.assertIn("ignoring unsafe restart record", result.stderr)
                    self.assertNotIn(f"start {READER}", box.calls())
                    self.assertEqual(box.active(READER), "inactive")
                    self.assertTrue((box.root / "elsewhere/units").exists() or make is not symlinked)

    def test_the_record_must_belong_to_the_user_running_the_script(self):
        script = (DEBIAN / "postinst").read_text()
        self.assertIn('[[ "${owner}" == "${EUID}"', script)
        self.assertIn('[[ "${owner}" == "${EUID}"', (DEBIAN / "prerm").read_text())

    def test_only_allowlisted_units_are_started_from_the_record(self):
        box = self.sandbox()
        box.installed()
        box.write_record("sshd.service", "../x.service", TARGETS, MOUNTS, READY, f"{CORE} sideways",
                         f"{CORE} enabled extra", READER)
        result = box.run("postinst", "abort-upgrade", "1.0")
        self.assertOk(result)
        self.assertEqual(box.manager(), ["daemon-reload", f"start {READER}"])
        for name in ("sshd.service", TARGETS, MOUNTS, READY):
            self.assertIn(f"unexpected restart record entry: {name} enabled", result.stderr)
        self.assertIn(f"unexpected restart record entry: {CORE} sideways", result.stderr)
        self.assertIn(f"unexpected restart record entry: {CORE} enabled extra", result.stderr)

    def test_a_record_without_a_valid_first_line_is_ignored(self):
        for header in (None, "install", "upgrade", "upgrade 1.0 x", "upgrade 1.0;x"):
            with self.subTest(header=header):
                box = self.sandbox()
                box.installed()
                box.write_record(READER, header=header)
                result = box.run("postinst", "abort-upgrade", "1.0")
                self.assertOk(result)
                self.assertIn("without a valid first line", result.stderr)
                self.assertNotIn(f"start {READER}", box.calls())
                self.assertFalse(box.record_exists())
        script = (DEBIAN / "postinst").read_text()
        allowed = re.search(r"^RESTARTED_UNITS=\(([^)]*)\)$", script, re.M)[1].split()
        self.assertEqual(allowed, [*ENABLED, READER])

    def test_every_abort_restarts_exactly_what_prerm_stopped(self):
        for args in ABORTS:
            with self.subTest(args=args):
                box = self.sandbox()
                box.installed(running=[*ENABLED, READER, TARGETS])
                self.assertOk(box.run("prerm", "upgrade", "1.0.0"))
                box.clear()
                self.assertOk(box.run("postinst", *args))
                # Reverse of the stop order: the reader and executors before core.
                self.assertEqual(
                    box.manager(),
                    ["daemon-reload", f"start {READER}", f"start {EXECUTORS[1]}",
                     f"start {EXECUTORS[0]}", f"start {CORE}"],
                )
                self.assertStates(box, [*ENABLED, READER])
                self.assertFalse(box.record_exists())
                box.clear()
                # A second abort has nothing left to restart.
                self.assertOk(box.run("postinst", *args))
                self.assertEqual(box.manager(), ["daemon-reload"])

    def test_abort_skips_a_unit_disabled_since_it_was_stopped(self):
        box = self.sandbox()
        box.installed(running=ENABLED)
        self.assertOk(box.run("prerm", "remove"))
        box.set(CORE, unitfile="disabled")
        result = box.run("postinst", "abort-remove")
        self.assertOk(result)
        self.assertIn(f"not starting {CORE}", result.stderr)
        self.assertStates(box, EXECUTORS)

    def test_abort_restarts_a_unit_that_was_disabled_but_running(self):
        box = self.sandbox()
        box.installed(running=ENABLED)
        box.set(CORE, unitfile="disabled")
        self.assertOk(box.run("prerm", "remove"))
        result = box.run("postinst", "abort-remove")
        self.assertOk(result)
        self.assertNotIn("not starting", result.stderr)
        self.assertStates(box, ENABLED)

    def test_configure_consumes_the_record_before_anything_can_fail(self):
        for failing in (f"limeosctl check-config {{root}}/etc/limeos/core.json",
                        f"restart {EXECUTORS[1]} {EXECUTORS[0]} {CORE}"):
            with self.subTest(failing=failing):
                box = self.sandbox()
                box.installed(running=[*ENABLED, READER])
                self.assertOk(box.run("prerm", "upgrade", "1.0"))
                box.fail(failing.format(root=box.root))
                self.assertFailed(box.run("postinst", "configure"))
                self.assertFalse(box.record_exists())
                self.assertNotIn(f"start {READER}", box.calls())

    def test_abort_without_a_running_manager_calls_nothing_and_ends_the_transaction(self):
        box = self.sandbox(systemd=False)
        box.installed()
        box.write_record(CORE)
        self.assertOk(box.run("postinst", "abort-remove"))
        self.assertEqual(box.calls(), [])
        self.assertFalse(box.record_exists())

    def test_abort_with_a_failing_start_starts_the_rest_and_ends_the_transaction(self):
        box = self.sandbox()
        box.installed(running=[*ENABLED, READER])
        self.assertOk(box.run("prerm", "remove"))
        box.fail(f"start {READER}")
        result = box.run("postinst", "abort-remove")
        self.assertFailed(result)
        self.assertIn(f"systemctl start {READER}", result.stderr)
        self.assertStates(box, ENABLED)
        self.assertFalse(box.record_exists())

    def test_unknown_argument_fails_without_effect(self):
        for args in (("bogus",), ("triggered", "x"), ()):
            with self.subTest(args=args):
                box = self.sandbox()
                box.installed()
                result = box.run("postinst", *args)
                self.assertFailed(result)
                self.assertIn("unknown argument", result.stderr)
                self.assertEqual(box.calls(), [])


class UnwindTests(SandboxTest):
    """Debian's call sequences, both packages being this version."""

    RUNNING = [*ENABLED, READER, TARGETS]

    def step(self, box, script, *args, ok=True, active=None, record=None):
        result = box.run(script, *args)
        (self.assertOk if ok else self.assertFailed)(result)
        if active is not None:
            self.assertStates(box, active)
        if record is None:
            self.assertFalse(box.record_exists(), f"{script} {args}")
        else:
            self.assertEqual(box.record(), record, f"{script} {args}")
        return result

    def test_upgrade(self):
        box = self.sandbox()
        box.installed(running=self.RUNNING)
        recorded = [*ENABLED, READER]
        self.step(box, "prerm", "upgrade", "1.1", active=[], record=recorded)
        self.step(box, "postrm", "upgrade", "1.1", active=[], record=recorded)
        self.step(box, "postinst", "configure", "1.0", active=[*ENABLED, READER])
        self.assertTrue(box.data_intact())

    def test_old_prerm_fails_then_failed_upgrade_succeeds(self):
        box = self.sandbox()
        box.installed(running=self.RUNNING)
        box.fail(f"stop {EXECUTORS[1]}")
        self.step(box, "prerm", "upgrade", "1.1", ok=False,
                  active=[EXECUTORS[1], READER, TARGETS], record=ENABLED)
        box.heal()
        self.step(box, "prerm", "failed-upgrade", "1.0", "1.1", active=[], record=[*ENABLED, READER])
        self.step(box, "postrm", "upgrade", "1.1", active=[], record=[*ENABLED, READER])
        self.step(box, "postinst", "configure", "1.0", active=[*ENABLED, READER])

    def test_old_prerm_and_failed_upgrade_both_fail_then_abort_upgrade(self):
        box = self.sandbox()
        box.installed(running=self.RUNNING)
        box.fail(f"stop {EXECUTORS[1]}")
        self.step(box, "prerm", "upgrade", "1.1", ok=False,
                  active=[EXECUTORS[1], READER, TARGETS], record=ENABLED)
        self.step(box, "prerm", "failed-upgrade", "1.0", "1.1", ok=False,
                  active=[EXECUTORS[1], READER, TARGETS], record=ENABLED)
        box.heal()
        self.step(box, "postinst", "abort-upgrade", "1.1", active=[*ENABLED, READER, TARGETS])

    def test_unpack_fails_after_prerm_upgrade(self):
        box = self.sandbox()
        box.installed(running=self.RUNNING)
        recorded = [*ENABLED, READER]
        self.step(box, "prerm", "upgrade", "1.1", active=[], record=recorded)
        self.step(box, "postrm", "abort-upgrade", "1.0", "1.1", active=[], record=recorded)
        self.step(box, "postinst", "abort-upgrade", "1.1", active=[*ENABLED, READER])

    def test_remove_twice_then_purge(self):
        box = self.sandbox()
        box.installed(running=self.RUNNING)
        self.step(box, "prerm", "remove", active=[], record=[*ENABLED, READER])
        self.step(box, "postrm", "remove", active=[])
        # dpkg deleted the unit files; a second removal runs both again.
        for name in ALL:
            box.unit(name, load="not-found")
        box.clear()
        self.step(box, "prerm", "remove", active=[], record=[])
        self.step(box, "postrm", "remove", active=[])
        self.step(box, "postrm", "purge", active=[])
        self.assertEqual(box.manager(), ["daemon-reload", "daemon-reload"])
        self.assertTrue(box.data_intact())
        for name in ENABLED:
            self.assertFalse((box.root / WANTS / name).is_symlink())

    def test_remove_fails_then_abort_remove(self):
        box = self.sandbox()
        box.installed(running=self.RUNNING)
        box.surviving(READER)
        self.step(box, "prerm", "remove", ok=False, record=[*ENABLED, READER])
        self.assertEqual(box.active(READER), "failed")
        self.assertEqual([box.active(n) for n in [*ENABLED, TARGETS]], ["inactive"] * 3 + ["active"])
        box.set(READER, active="active")
        self.step(box, "postinst", "abort-remove", active=[*ENABLED, READER, TARGETS])

    def test_deconfigure_fails_then_abort_deconfigure(self):
        box = self.sandbox()
        box.installed(running=self.RUNNING)
        box.fail(f"stop {CORE}")
        args = ("in-favour", "other", "1.0", "removing", "conflict", "2.0")
        self.step(box, "prerm", "deconfigure", *args, ok=False, active=self.RUNNING, record=[CORE])
        box.heal()
        self.step(box, "postinst", "abort-deconfigure", *args, active=self.RUNNING)

    def test_deconfigure_then_configure(self):
        box = self.sandbox()
        box.installed(running=self.RUNNING)
        args = ("in-favour", "other", "1.0", "removing", "conflict", "2.0")
        self.step(box, "prerm", "deconfigure", *args, active=[], record=[*ENABLED, READER])
        self.step(box, "postinst", "configure", "1.0", active=[*ENABLED, READER])


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

    def test_prerm_and_postinst_agree_on_the_restarted_units(self):
        lists = [re.search(r"^RESTARTED_UNITS=\(([^)]*)\)$", (DEBIAN / s).read_text(), re.M)[1]
                 for s in ("prerm", "postinst")]
        self.assertEqual(lists[0], lists[1])

    def test_shadow_profile_scopes_the_record_and_unit_names(self):
        build = load_build()
        for script in ("prerm", "postrm", "postinst"):
            with self.subTest(script=script):
                shadow = build.profile_text((DEBIAN / script).read_text(), True)
                self.assertNotIn("/run/limeos-package", shadow)
                self.assertIn("/run/limeos-shadow-package", shadow)
                for name in ALL:
                    self.assertNotIn(f" {name}", shadow)
                    self.assertNotIn(f"({name}", shadow)

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
