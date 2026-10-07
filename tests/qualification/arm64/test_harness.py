#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Local harness regression tests. No SSH, guests or installed-service mutations."""

import copy
import json
import os
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

import guest_supervisor
import native_vm
import qualification as q
import record_run
from native_vm import Guest
from record_run import budget_table

HERE = Path(__file__).resolve().parent


def build_fixture():
    identity = {
        "source_commit": "a" * 40,
        "fixtures_commit": "b" * 40,
        "package_version": "0.4.4+arm64.1",
        "architecture": "arm64",
        "authority_schema": 8,
    }
    return {
        "identity": identity,
        "steps": [
            {"step": step, "exit": 0}
            for step in (
                "apt",
                "rustup",
                "fetch",
                "fmt",
                "clippy",
                "test",
                "contracts",
                "dependency-direction",
                "release",
                "package-standard",
                "package-shadow",
                "signing-key",
                "repository",
            )
        ],
        "binaries": {n: {"sha256": "c" * 64, "machine": "AArch64"} for n in q.BINARIES},
        "packages": {
            f"{n}_0.4.4+arm64.1_arm64.deb": "d" * 64
            for n in ("limeos", "limeos-shadow")
        },
        "tests": {"passed": 185, "failed": 0},
        "package_control": {
            f"{n}_0.4.4+arm64.1_arm64.deb": f"Package: {n}\nVersion: 0.4.4+arm64.1\nArchitecture: arm64\n"
            for n in ("limeos", "limeos-shadow")
        },
    }


class IdentityTests(unittest.TestCase):
    def test_production_wybie_is_rejected_before_any_ssh(self):
        with patch("native_vm.subprocess.run") as run:
            for host in (
                "wybie",
                "holly@wybie",
                "Wybie.local",
                "root@WYBIE.example.test",
            ):
                with (
                    self.subTest(host=host),
                    self.assertRaisesRegex(ValueError, "production"),
                ):
                    Guest("run", host)
            run.assert_not_called()

    def test_rejects_mixed_package_architecture_failed_gate_and_incomplete_build(self):
        original = build_fixture()
        mutations = []
        failed = copy.deepcopy(original)
        failed["steps"][0]["exit"] = 1
        mutations.append(failed)
        missing = copy.deepcopy(original)
        missing["steps"].pop()
        mutations.append(missing)
        relabeled = copy.deepcopy(original)
        relabeled["identity"]["package_version"] = "0.4.2"
        mutations.append(relabeled)
        x86 = copy.deepcopy(original)
        x86["binaries"]["limeos-core"]["machine"] = "X86-64"
        mutations.append(x86)
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "build.json"
            path.write_text(json.dumps(original))
            self.assertEqual(q.load_build(path), original)
            for mutation in mutations:
                with self.subTest(mutation=mutation):
                    path.write_text(json.dumps(mutation))
                    with self.assertRaises(ValueError):
                        q.load_build(path)

    def test_file_hash_drift_and_escape_are_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "source.rs").write_text("frozen")
            recorded = q.sha(root / "source.rs")
            self.assertEqual(q.verify_files(root, {"source.rs": recorded}), 1)
            (root / "source.rs").write_text("edited")
            with self.assertRaises(ValueError):
                q.verify_files(root, {"source.rs": recorded})
            with self.assertRaises(ValueError):
                q.verify_files(root, {"../outside": recorded})
            (root / "alias").symlink_to(root / "source.rs")
            with self.assertRaises(ValueError):
                q.verify_files(root, {"alias": q.sha(root / "source.rs")})

    def test_source_schema_must_match_manifest(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            file = root / "source/crates/persistence/src/lib.rs"
            file.parent.mkdir(parents=True)
            file.write_text("pub const SCHEMA_VERSION: u32 = 8;")
            (root / "source/frontend").mkdir()
            (root / "source/frontend/index.html").write_text("UI")
            (root / "fixtures").mkdir()
            (root / "fixtures/test.py").write_text("fixture")
            manifest = {
                "identity": {"authority_schema": 6},
                "runtime_source_sha256": {"crates/persistence/src/lib.rs": q.sha(file)},
                "frontend_dist_sha256": {
                    "frontend/index.html": q.sha(root / "source/frontend/index.html")
                },
                "fixtures_sha256": {"test.py": q.sha(root / "fixtures/test.py")},
            }
            with self.assertRaises(ValueError):
                q.verify_source(root, manifest)

    def test_disposable_marker_does_not_allow_emulation(self):
        with (
            patch.object(q.os, "geteuid", return_value=0),
            patch.object(q.socket, "gethostname", return_value="limeos-p01-test"),
            patch.object(q.platform, "machine", return_value="aarch64"),
            patch.object(q.subprocess, "check_output", return_value="qemu\n"),
            self.assertRaisesRegex(RuntimeError, "native KVM"),
        ):
            q.guest_guard()

    def test_guest_state_cannot_be_used_on_a_different_host(self):
        with (
            tempfile.TemporaryDirectory() as directory,
            patch("native_vm.STATE", Path(directory)),
        ):
            local = Path(directory) / "run"
            local.mkdir()
            (local / "guest.json").write_text(
                json.dumps({"host": "test-a", "port": 22801})
            )
            with self.assertRaises(ValueError):
                Guest("run", "test-b")
            for name in ("../escape", "x;touch file", "-bad"):
                with self.assertRaises(ValueError):
                    Guest(name, "test-a")


class EvidenceTests(unittest.TestCase):
    def test_evidence_cannot_overwrite_history_or_bound_build_artifact(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            manifest = {
                "identity": build_fixture()["identity"],
                "runtime_source_sha256": {"a": "a" * 64},
                "frontend_dist_sha256": {"b": "b" * 64},
                "fixtures_sha256": {"c": "c" * 64},
            }
            source = root / "source.json"
            source.write_text(json.dumps(manifest))
            build = build_fixture()
            build.update(
                {
                    k: manifest[k]
                    for k in (
                        "runtime_source_sha256",
                        "frontend_dist_sha256",
                        "fixtures_sha256",
                    )
                }
            )
            build["source_manifest_sha256"] = q.sha(source)
            result = root / "build.json"
            result.write_text(json.dumps(build))
            run = root / "docs/rewrite-evidence/arm64/run"
            arguments = [
                "record_run.py",
                "--run",
                "run",
                "--source-manifest",
                str(source),
                "--build-result",
                str(result),
            ]
            with (
                patch.object(record_run, "ROOT", root),
                patch.object(
                    sys, "argv", [*arguments, "--artifact", str(root) + "=build"]
                ),
                self.assertRaises(SystemExit) as caught,
            ):
                record_run.main()
            self.assertEqual(caught.exception.code, 2)
            self.assertFalse(run.exists())
            run.mkdir(parents=True)
            sentinel = run / "historical.json"
            sentinel.write_text("historical bytes")
            with (
                patch.object(record_run, "ROOT", root),
                patch.object(sys, "argv", arguments),
                self.assertRaises(SystemExit) as caught,
            ):
                record_run.main()
            self.assertEqual(caught.exception.code, 2)
            self.assertEqual(sentinel.read_text(), "historical bytes")

    def test_missing_write_counters_are_unavailable_and_short_idle_is_not_qualified(
        self,
    ):
        window = {
            "seconds": 600,
            "limeos_cpu_percent_of_one_core": 0.01,
            "per_unit": {
                "limeos-core": {"process_write_bytes": 4096},
                "limeos-storaged": {},
            },
        }
        install = {
            "checks": [
                {
                    "name": "idle CPU and bytes written over 600 seconds, no subscribers",
                    "result": "pass",
                    "observation": window,
                }
            ]
        }
        rows = budget_table(install)
        self.assertEqual(rows[-1][1], "unavailable")
        self.assertIsNone(rows[-1][-1])
        window["seconds"] = 30
        self.assertEqual(budget_table(install), [])

    def test_all_entry_points_offer_help_without_guest_access(self):
        for name in (
            "build_guest",
            "install_guest",
            "upgrade_guest",
            "extra_guest",
            "approved_guest",
            "native_vm",
            "guest_supervisor",
            "make_bundle",
            "record_run",
        ):
            with self.subTest(script=name):
                result = subprocess.run(
                    [sys.executable, str(HERE / (name + ".py")), "--help"],
                    capture_output=True,
                    text=True,
                    timeout=10,
                    check=False,
                )
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertIn("usage:", result.stdout)


def write_authorization(directory, **overrides):
    record = {
        "version": 1,
        "host": "holly@wybie",
        "not_before": "2026-10-06T21:30:00Z",
        "not_after": "2026-10-07T06:00:00Z",
        "authorized_by": "operator",
        "reference": "docs/plans/2026-10-06-engineer-arm64-overnight-qualification.md",
        "scope": "disposable Debian ARM64 KVM guests only",
    }
    record.update(overrides)
    for key in [k for k, v in record.items() if v is None]:
        del record[key]
    path = Path(directory) / "authorization.json"
    path.write_text(json.dumps(record))
    return path


INSIDE = native_vm.parse_utc("2026-10-06T23:00:00Z")


class AuthorizationTests(unittest.TestCase):
    def test_dispatch_timeouts_end_before_the_authorized_cutoff(self):
        with tempfile.TemporaryDirectory() as directory:
            now = native_vm.parse_utc("2026-10-07T05:59:55Z")
            with (
                patch("native_vm.STATE", Path(directory)),
                patch("native_vm.utc_now", return_value=now),
                patch("native_vm.subprocess.run") as dispatched,
            ):
                guest = Guest("bounded", "holly@wybie", write_authorization(directory))
                guest.port = 22801
                for action in (
                    lambda: guest.host_run("true", timeout=60),
                    lambda: guest.ssh("true"),
                    lambda: guest.push("a", "b"),
                    lambda: guest.pull("a", Path(directory) / "b"),
                    lambda: guest.host_push("a", "b"),
                    lambda: guest.host_pull("a", Path(directory) / "b"),
                ):
                    action()
                    self.assertEqual(dispatched.call_args.kwargs["timeout"], 4)
                guest.host_run("true", timeout=0.5)
                self.assertEqual(dispatched.call_args.kwargs["timeout"], 0.5)
                dispatched.reset_mock()
                with (
                    patch(
                        "native_vm.utc_now",
                        return_value=native_vm.parse_utc("2026-10-07T05:59:59Z"),
                    ),
                    self.assertRaisesRegex(SystemExit, "too short"),
                ):
                    guest.host_pull("a", Path(directory) / "b")
                dispatched.assert_not_called()

    def test_boot_rechecks_expiry_after_preparation_before_host_copy(self):
        with tempfile.TemporaryDirectory() as directory:
            now = [INSIDE]
            commands = []
            path = write_authorization(directory)

            def dispatch(argv, **kwargs):
                commands.append(argv)
                if argv[0] == "ssh-keygen":
                    Path(argv[-1]).with_suffix(".pub").write_text("fixture public key")
                if argv[0] == "ssh" and argv[-1].startswith("mkdir -p "):
                    now[0] = native_vm.parse_utc("2026-10-07T06:00:00Z")
                return subprocess.CompletedProcess(
                    argv, 0, stdout="a" * 128 + " image\n"
                )

            args = SimpleNamespace(
                name="expiry-boot",
                host="holly@wybie",
                authorization=path,
                terminate_at=None,
                port=22801,
                cpus=1,
                memory=512,
                disk="1G",
                image_sha512="a" * 128,
                storage_disks=False,
                timeout=60,
            )
            with (
                patch("native_vm.STATE", Path(directory)),
                patch("native_vm.utc_now", side_effect=lambda: now[0]),
                patch("native_vm.host_info", return_value={}),
                patch("native_vm.subprocess.run", side_effect=dispatch),
                self.assertRaisesRegex(SystemExit, "expired"),
            ):
                native_vm.boot(args)
            self.assertFalse(any(command[0] == "scp" for command in commands))

    def test_stop_rechecks_expiry_before_collecting_host_logs(self):
        with tempfile.TemporaryDirectory() as directory:
            now = [INSIDE]
            commands = []
            path = write_authorization(directory)
            local = Path(directory) / "expiry-stop"
            local.mkdir()
            (local / "guest.json").write_text(
                json.dumps(
                    {
                        "host": "holly@wybie",
                        "port": 22801,
                        "marker": "exact-marker",
                    }
                )
            )

            def dispatch(argv, **kwargs):
                commands.append(argv)
                if argv[0] == "ssh":
                    now[0] = native_vm.parse_utc("2026-10-07T06:00:00Z")
                return subprocess.CompletedProcess(argv, 0, stdout="")

            with (
                patch("native_vm.STATE", Path(directory)),
                patch("native_vm.utc_now", side_effect=lambda: now[0]),
                patch("native_vm.host_info", return_value={}),
                patch("native_vm.subprocess.run", side_effect=dispatch),
                self.assertRaisesRegex(SystemExit, "expired"),
            ):
                native_vm.stop(
                    SimpleNamespace(
                        name="expiry-stop",
                        host="holly@wybie",
                        authorization=path,
                    )
                )
            self.assertFalse(any(command[0] == "scp" for command in commands))

    def test_aliases_and_addresses_of_production_are_production(self):
        def fake_ssh_g(argv, **kwargs):
            host = argv[-1].rsplit("@", 1)[-1]
            name = "wybie" if host == "pi" else host
            return subprocess.CompletedProcess(
                argv, 0, stdout=f"user holly\nhostname {name}\n"
            )

        def fake_addresses(host, *rest):
            table = {
                "wybie": "100.119.146.6",
                "100.119.146.6": "100.119.146.6",
                "spare": "10.0.0.9",
            }
            return [(0, 0, 0, "", (table.get(host, "192.0.2.1"), 0))]

        with (
            patch("native_vm.subprocess.run", side_effect=fake_ssh_g),
            patch("native_vm.socket.getaddrinfo", side_effect=fake_addresses),
        ):
            self.assertTrue(native_vm.production_host("holly@pi"))
            self.assertTrue(native_vm.production_host("holly@100.119.146.6"))
            self.assertFalse(native_vm.production_host("holly@spare"))
            with self.assertRaisesRegex(ValueError, "production"):
                Guest("run", "holly@100.119.146.6")

    def test_named_window_admits_only_the_exact_host_without_ssh(self):
        with (
            tempfile.TemporaryDirectory() as directory,
            patch("native_vm.subprocess.run") as run,
            patch("native_vm.utc_now", return_value=INSIDE),
        ):
            path = write_authorization(directory)
            guest = Guest("run", "holly@wybie", path)
            self.assertEqual(guest.authorization["not_after"], "2026-10-07T06:00:00Z")
            self.assertEqual(len(guest.authorization["sha256"]), 64)
            for host in ("wybie", "root@wybie", "holly@wybie.local"):
                with (
                    self.subTest(host=host),
                    self.assertRaisesRegex(ValueError, "authorization names"),
                ):
                    Guest("run", host, path)
            run.assert_not_called()

    def test_absent_malformed_expired_or_overlong_windows_are_refused(self):
        cases = {
            "absent": None,
            "missing key": {"scope": None},
            "extra key": {"extra": "x"},
            "version": {"version": 2},
            "blank owner": {"authorized_by": " "},
            "offset time": {"not_after": "2026-10-07T06:00:00+00:00"},
            "spaced time": {"not_before": "2026-10-06 21:30:00"},
            "inverted": {"not_before": "2026-10-07T06:00:00Z"},
            "overlong": {"not_before": "2026-10-06T17:59:59Z"},
            "expired": {
                "not_before": "2026-10-06T10:00:00Z",
                "not_after": "2026-10-06T22:00:00Z",
            },
            "not started": {"not_before": "2026-10-06T23:30:00Z"},
        }
        with (
            tempfile.TemporaryDirectory() as directory,
            patch("native_vm.subprocess.run") as run,
            patch("native_vm.utc_now", return_value=INSIDE),
        ):
            for label, overrides in cases.items():
                with self.subTest(label), self.assertRaises(ValueError):
                    path = (
                        None
                        if overrides is None
                        else write_authorization(directory, **overrides)
                    )
                    Guest("run", "holly@wybie", path)
            (Path(directory) / "broken.json").write_text("{")
            with self.assertRaises(ValueError):
                Guest("run", "holly@wybie", Path(directory) / "broken.json")
            run.assert_not_called()

    def test_expiry_during_a_run_blocks_every_further_ssh(self):
        with (
            tempfile.TemporaryDirectory() as directory,
            patch("native_vm.STATE", Path(directory)),
            patch("native_vm.subprocess.run") as run,
        ):
            path = write_authorization(directory)
            with patch("native_vm.utc_now", return_value=INSIDE):
                guest = Guest("run", "holly@wybie", path)
            guest.port = 22801
            with patch(
                "native_vm.utc_now",
                return_value=native_vm.parse_utc("2026-10-07T06:00:00Z"),
            ):
                for action in (
                    lambda: guest.host_run("true"),
                    lambda: guest.ssh("true"),
                    lambda: guest.push("a", "b"),
                    lambda: guest.pull("a", Path(directory) / "b"),
                ):
                    with self.assertRaisesRegex(SystemExit, "expired"):
                        action()
            run.assert_not_called()

    def test_guest_deadlines_stay_inside_the_window(self):
        with (
            tempfile.TemporaryDirectory() as directory,
            patch("native_vm.subprocess.run"),
            patch("native_vm.utc_now", return_value=INSIDE),
        ):
            guest = Guest("run", "holly@wybie", write_authorization(directory))
            terminate, kill = native_vm.deadlines(guest, None)
            self.assertEqual(
                (terminate, kill),
                (
                    native_vm.parse_utc("2026-10-07T05:55:00Z"),
                    native_vm.parse_utc("2026-10-07T05:57:00Z"),
                ),
            )
            early = native_vm.parse_utc("2026-10-06T23:05:00Z")
            self.assertEqual(
                native_vm.deadlines(guest, early),
                (early, native_vm.parse_utc("2026-10-06T23:07:00Z")),
            )
            late = native_vm.parse_utc("2026-10-07T05:41:00Z")
            with self.assertRaisesRegex(SystemExit, "15 minutes"):
                native_vm.deadlines(guest, None, now=late)
            with self.assertRaisesRegex(SystemExit, "no usable time"):
                native_vm.deadlines(guest, INSIDE)
        with (
            patch("native_vm.production_host", return_value=False),
            self.assertRaisesRegex(SystemExit, "terminate-at"),
        ):
            native_vm.deadlines(Guest("run", "isolated"), None)


TARGET = (
    "import signal, sys, time\n"
    "if sys.argv[1] == 'stubborn': signal.signal(signal.SIGTERM, signal.SIG_IGN)\n"
    "time.sleep(float(sys.argv[2]))\n"
)


def spawn(mode, marker, seconds=60):
    return subprocess.Popen([sys.executable, "-c", TARGET, mode, str(seconds), marker])


def start_ticks(pid):
    stat = Path(f"/proc/{pid}/stat").read_text()
    return int(stat[stat.rindex(")") + 2 :].split()[19])


def supervise(target, log, terminate_in, kill_in, marker=None, ticks=None):
    now = time.time()
    return subprocess.run(
        [
            sys.executable,
            str(HERE / "guest_supervisor.py"),
            "--pid",
            str(target.pid),
            "--start-ticks",
            str(ticks if ticks is not None else start_ticks(target.pid)),
            "--uid",
            str(os.getuid()),
            "--marker",
            marker or target.args[-1],
            "--terminate-at",
            str(now + terminate_in),
            "--kill-at",
            str(now + kill_in),
            "--log",
            str(log),
            "--poll",
            "0.2",
        ],
        timeout=30,
        check=False,
    )


def records(log):
    return [json.loads(line) for line in Path(log).read_text().splitlines()]


def events(log):
    return [record["event"] for record in records(log)]


class SupervisorTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.log = Path(self.directory.name) / "supervisor.log"
        self.processes = []

    def tearDown(self):
        for process in self.processes:
            process.kill()
            process.wait()
        self.directory.cleanup()

    def start(self, mode, marker, seconds=60):
        process = spawn(mode, marker, seconds)
        self.processes.append(process)
        time.sleep(0.3)
        return process

    def process_state(self, guest, **overrides):
        state = {
            "pid": guest.pid,
            "start_ticks": start_ticks(guest.pid),
            "uid": os.getuid(),
            "marker": guest.args[-1],
        }
        state.update(overrides)
        path = Path(self.directory.name) / "process.json"
        path.write_text(json.dumps(state))
        return path

    def test_orderly_stop_matches_exact_marker_and_spares_substring(self):
        guest = self.start("cooperative", "limeos-arm64-exact")
        other = self.start("cooperative", "limeos-arm64-exact-suffix")
        self.assertEqual(
            guest_supervisor.stop_record(
                self.process_state(other, marker="limeos-arm64-exact"),
                grace=0.1,
            ),
            2,
        )
        self.assertIsNone(other.poll())
        self.assertEqual(guest_supervisor.stop_record(self.process_state(guest)), 0)
        self.assertEqual(guest.wait(timeout=5), -15)
        self.assertIsNone(other.poll())

    def test_orderly_stop_refuses_reused_pid_and_wrong_uid(self):
        guest = self.start("cooperative", "limeos-arm64-reused")
        for mismatch in ({"start_ticks": 1}, {"uid": os.getuid() + 1}):
            with self.subTest(**mismatch):
                self.assertEqual(
                    guest_supervisor.stop_record(
                        self.process_state(guest, **mismatch),
                        grace=0.1,
                    ),
                    2,
                )
                self.assertIsNone(guest.poll())

    def test_orderly_stop_pidfd_cannot_signal_a_replacement_process(self):
        original = self.start("cooperative", "limeos-arm64-old")
        pidfd = os.pidfd_open(original.pid)
        original.kill()
        original.wait()
        replacement = self.start("cooperative", "limeos-arm64-replacement")
        try:
            # Simulate PID reuse between pidfd_open and /proc identity reads.
            with patch("guest_supervisor.os.pidfd_open", return_value=pidfd):
                self.assertEqual(
                    guest_supervisor.stop_record(
                        self.process_state(replacement),
                        grace=0.1,
                    ),
                    0,
                )
            self.assertIsNone(replacement.poll())
        finally:
            # stop_record closes the descriptor even when its process is gone.
            try:
                os.close(pidfd)
            except OSError:
                pass

    def test_deadline_forces_only_the_bound_guest_and_spares_others(self):
        guest = self.start("stubborn", "limeos-arm64-run-aaaa")
        unrelated = self.start("stubborn", "limeos-arm64-other-bbbb")
        result = supervise(guest, self.log, 0.5, 2.0)
        self.assertEqual(result.returncode, 0)
        self.assertEqual(
            events(self.log), ["bound", "sigterm", "sigkill", "guest_gone"]
        )
        self.assertEqual(guest.wait(timeout=5), -9)
        self.assertIsNone(unrelated.poll(), "an unrelated process was signalled")
        bound, sigterm, sigkill, _ = records(self.log)
        self.assertGreaterEqual(sigterm["epoch"], round(bound["terminate_at"], 3))
        self.assertGreaterEqual(sigkill["epoch"], round(bound["kill_at"], 3))

    def test_cooperative_guest_stops_at_sigterm(self):
        guest = self.start("cooperative", "limeos-arm64-run-cccc")
        self.assertEqual(supervise(guest, self.log, 0.5, 10).returncode, 0)
        self.assertEqual(events(self.log), ["bound", "sigterm", "guest_gone"])
        self.assertEqual(guest.wait(timeout=5), -15)

    def test_mismatched_identity_is_never_signalled(self):
        guest = self.start("stubborn", "limeos-arm64-run-dddd")
        for kwargs in ({"marker": "limeos-arm64-run-other"}, {"ticks": 1}):
            with self.subTest(**kwargs):
                self.assertEqual(
                    supervise(guest, self.log, 0.1, 0.5, **kwargs).returncode, 2
                )
                self.assertIsNone(guest.poll())
        self.assertEqual(set(events(self.log)), {"refused_unbound"})

    def test_guest_stopping_first_ends_supervision_without_signals(self):
        guest = self.start("cooperative", "limeos-arm64-run-eeee", seconds=0.8)
        started = time.monotonic()
        self.assertEqual(supervise(guest, self.log, 60, 120).returncode, 0)
        self.assertLess(time.monotonic() - started, 10)
        self.assertEqual(events(self.log), ["bound", "guest_gone"])
        self.assertEqual(guest.wait(timeout=5), 0)


if __name__ == "__main__":
    unittest.main()
