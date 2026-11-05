#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Local regressions for the Pi night orchestrator and host helper. No SSH, no guests."""

import hashlib
import json
import tempfile
import unittest
from datetime import datetime, timedelta, timezone
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

import native_vm
import pi_night_host as ph
import run_pi_night as night

GIB = 1024 * 1024


def mem(available=6 * GIB, swap_total=2 * GIB, swap_used=0):
    return {
        "MemAvailable": available,
        "SwapTotal": swap_total,
        "SwapFree": swap_total - swap_used,
    }


class GuardTests(unittest.TestCase):
    def test_start_thresholds(self):
        self.assertEqual(ph.guard_failures(mem(), "0.5 0.5 0.5 1/1 1", "start"), [])
        self.assertTrue(ph.guard_failures(mem(available=3 * GIB - 1), "0 0 0", "start"))
        self.assertEqual(
            ph.guard_failures(mem(available=3 * GIB), "0 0 0", "start"), []
        )
        self.assertTrue(ph.guard_failures(mem(), "2.01 0.1 0.1", "start"))
        self.assertTrue(ph.guard_failures(mem(), "0.1 2.5 0.1", "start"))
        self.assertTrue(ph.guard_failures(mem(), "garbage", "start"))

    def test_swap_is_at_least_as_strict_as_the_previous_run(self):
        # 450 MiB absolute (previous run) and 90% fraction (new request).
        self.assertTrue(ph.guard_failures(mem(swap_used=451 * 1024), "0 0 0", "start"))
        self.assertEqual(
            ph.guard_failures(mem(swap_used=450 * 1024), "0 0 0", "start"), []
        )
        small = mem(swap_total=400 * 1024, swap_used=361 * 1024)
        self.assertTrue(ph.guard_failures(small, "0 0 0", "start"))
        self.assertEqual(ph.guard_failures(mem(swap_total=0), "0 0 0", "start"), [])

    def test_running_floor_keeps_the_previous_two_gib_and_ignores_load(self):
        self.assertTrue(ph.guard_failures(mem(available=2 * GIB - 1), "9 9 9", "run"))
        self.assertEqual(ph.guard_failures(mem(available=2 * GIB), "9 9 9", "run"), [])
        self.assertTrue(ph.guard_failures(mem(swap_used=500 * 1024), "0 0 0", "run"))


ROWS = "\n".join(
    [
        "aaa|/jellyfin|running|2026-10-01T00:00:00Z|0",
        "bbb|/mattermost|running|2026-10-02T00:00:00Z|1",
    ]
)
STOPPED = ROWS.replace("bbb|/mattermost|running", "bbb|/mattermost|exited")


class ContainerTests(unittest.TestCase):
    base = ph.parse_containers(ROWS)

    def test_unchanged(self):
        self.assertEqual(
            ph.container_failures(self.base, ph.parse_containers(ROWS)), []
        )

    def test_stop_vanish_recreate_restart(self):
        self.assertIn(
            "mattermost: state exited",
            ph.container_failures(self.base, ph.parse_containers(STOPPED))[0],
        )
        gone = ph.parse_containers("aaa|/jellyfin|running|2026-10-01T00:00:00Z|0")
        self.assertIn("no longer exists", ph.container_failures(self.base, gone)[0])
        recreated = ROWS.replace("bbb|", "ccc|")
        self.assertIn(
            "no longer exists",
            ph.container_failures(self.base, ph.parse_containers(recreated))[0],
        )
        restarted = ROWS.replace("2026-10-01T00:00:00Z", "2026-11-05T00:00:00Z")
        self.assertIn(
            "restarted",
            ph.container_failures(self.base, ph.parse_containers(restarted))[0],
        )

    def test_new_containers_are_not_failures(self):
        extra = ph.parse_containers(ROWS + "\nddd|/new|running|x|0")
        self.assertEqual(ph.container_failures(self.base, extra), [])


def snapshot(
    packages=("bash\t5", "curl\t8"),
    containers=ROWS,
    available=6 * GIB,
    swap_used=0,
    qemu=(),
    uid=1000,
    listeners=None,
):
    def command(out="", code=0):
        return {"exit": code, "stdout": out, "stderr": ""}

    listeners = listeners or (
        "State Recv-Q Send-Q Local Address:Port Peer Address:Port Process\n"
        'LISTEN 0 4096 0.0.0.0:22 0.0.0.0:* users:(("sshd",pid=1,fd=3))\n'
    )
    return {
        "uid": uid,
        "boot_id": "boot",
        "meminfo": mem(available=available, swap_used=swap_used),
        "loadavg": "0.1 0.1 0.1 1/1 1",
        "vmstat": {"oom_kill": 0},
        "packages": command("\n".join(packages) + "\n"),
        "manual_packages": command("bash\ncurl\n"),
        "running_units": command("ssh.service loaded active running OpenSSH\n"),
        "containers": command(containers),
        "docker_ps": command(
            "aaa\tjellyfin\trunning\t1d\tUp\nbbb\tmattermost\trunning\t1d\tUp\n"
        ),
        "listeners": command(listeners),
        "qemu_processes": list(qemu),
        "staging": command("", 2),
        "temperature_millidegrees": 50000,
    }


class PureTests(unittest.TestCase):
    def test_start_refusals(self):
        self.assertEqual(night.start_refusals(snapshot()), [])
        self.assertIn(
            "container mattermost is exited, not running",
            night.start_refusals(snapshot(containers=STOPPED)),
        )
        self.assertTrue(night.start_refusals(snapshot(containers="")))
        self.assertTrue(night.start_refusals(snapshot(qemu=[{"pid": 1}])))
        self.assertTrue(night.start_refusals(snapshot(available=GIB)))
        self.assertTrue(night.start_refusals(snapshot(uid=0)))
        broken = snapshot()
        broken["containers"] = {"exit": 1, "stdout": "", "stderr": "permission denied"}
        self.assertTrue(night.start_refusals(broken))

    def test_simulation_parsing_flags_upgrades_and_removals(self):
        text = "Inst qemu-utils (1 Debian)\nInst libc6 [2.36-8] (2.36-9 Debian)\nRemv foo:arm64 [1]\nConf x\n"
        self.assertEqual(
            night.parse_simulation(text), ({"qemu-utils"}, {"libc6"}, {"foo"})
        )

    def test_compare_snapshots(self):
        self.assertEqual(night.compare_snapshots(snapshot(), snapshot()), [])
        self.assertIn(
            "package list differs",
            night.compare_snapshots(snapshot(), snapshot(packages=("bash\t5",))),
        )
        extra = snapshot(
            listeners="State Recv-Q Send-Q Local Address:Port Peer\n"
            'LISTEN 0 1 127.0.0.1:22931 0.0.0.0:* users:(("qemu",pid=2,fd=3))\n'
        )
        self.assertIn(
            "listening sockets differ (ss -tlnp)",
            night.compare_snapshots(snapshot(), extra),
        )
        self.assertTrue(
            any(
                p.startswith("docker:")
                for p in night.compare_snapshots(
                    snapshot(), snapshot(containers=STOPPED)
                )
            )
        )
        self.assertTrue(
            night.compare_snapshots(snapshot(), snapshot(qemu=[{"pid": 5}]))
        )
        self.assertTrue(
            night.compare_snapshots(snapshot(), snapshot(swap_used=500 * 1024))
        )

    def test_deadline_and_plan(self):
        now = datetime(2026, 11, 5, 22, 0, tzinfo=timezone.utc)
        auth = {"end": now + timedelta(hours=8)}
        night.check_deadline(now + timedelta(hours=6), auth, now)
        for bad, why in (
            (now + timedelta(minutes=10), "no usable"),
            (now + timedelta(hours=7, minutes=50), "before the authorization"),
        ):
            with self.assertRaisesRegex(night.Refused, why):
                night.check_deadline(bad, auth, now)
        with self.assertRaises(night.Refused):
            night.check_deadline(now + timedelta(hours=6), None, now)
        every = ["install", "container", "stack", "scale"]
        self.assertEqual(night.plan_suites(every, now + timedelta(hours=9), now), every)
        self.assertEqual(
            night.plan_suites(every, now + timedelta(minutes=90), now),
            ["install", "container"],
        )

    def test_the_guest_is_bounded_and_deprioritised(self):
        self.assertLessEqual(night.MEMORY_MIB, 1536)
        self.assertEqual(night.CPUS, 2)
        source = Path(native_vm.__file__).read_text()
        for needle in ('"/usr/bin/nice"', '"19"', '"/usr/bin/ionice"', '"3"'):
            self.assertIn(needle, source)

    def test_host_commands_never_touch_docker_or_stacks(self):
        with (
            patch.object(native_vm, "Guest"),
            tempfile.TemporaryDirectory() as directory,
        ):
            host = night.Host("holly@wybie", Path("a.json"), Path(directory))
            for command in (
                "docker restart x",
                "ls /opt/stacks",
                "systemctl restart docker",
                "cd /opt/stacks && ls",
            ):
                with self.assertRaises(night.Refused):
                    host.run(command)

    def test_every_suite_runs_a_bundled_guest_script(self):
        for spec in night.SUITES.values():
            self.assertIn(
                "/root/qual/fixtures/tests/qualification/arm64/", spec["command"]
            )
        self.assertIn("p04_stack", (night.HERE / "stack_guest.py").read_text())


class FakeHost:
    """Just enough host to exercise ordering, refusals and teardown."""

    def __init__(self, **state):
        self.state = dict(
            packages={"bash": "5", "curl": "8"},
            containers=ROWS,
            available=6 * GIB,
            swap_used=0,
            abort=False,
            **state,
        )
        self.commands = []

    def snap(self):
        s = self.state
        packages = [f"{n}\t{v}" for n, v in sorted(s["packages"].items())]
        return snapshot(
            packages=packages,
            containers=s["containers"],
            available=s["available"],
            swap_used=s["swap_used"],
        )

    def run(self, command, timeout=60, input=None):
        self.commands.append(command)
        s = self.state

        def ok(out="", code=0):
            return SimpleNamespace(returncode=code, stdout=out, stderr="")

        if command.endswith("snapshot"):
            return ok(json.dumps(self.snap()))
        if "pi_night_host.py guard" in command:
            failures = ph.guard_failures(
                mem(s["available"], swap_used=s["swap_used"]), "0.1 0.1 0.1", "start"
            )
            return ok(json.dumps({"failures": failures}), 3 if failures else 0)
        if command.startswith("sha256sum"):
            return ok(
                hashlib.sha256(Path(ph.__file__).read_bytes()).hexdigest() + "  f\n"
            )
        if command.startswith("sha512sum"):
            return ok(night.IMAGE_SHA512 + "  f\n")
        if "apt-get -s install" in command:
            return ok(
                "".join(
                    f"Inst {n} (1)\n"
                    for n in ("qemu-system-arm", "qemu-utils", "libfoo")
                    if n not in s["packages"]
                )
            )
        if "apt-get install -y" in command:
            s["packages"].update(
                {"qemu-system-arm": "1", "qemu-utils": "1", "libfoo": "1"}
            )
            return ok()
        if "apt-get -s purge" in command:
            return ok(
                "".join(f"Remv {n} [1]\n" for n in command.split("purge ")[1].split())
            )
        if "apt-get purge -y" in command:
            for n in command.split("purge -y ")[1].split():
                s["packages"].pop(n, None)
            return ok()
        if command.startswith("cat") and "watchdog.jsonl" in command:
            return ok('{"event": "watchdog_started"}\n')
        if command.startswith("test -e") and command.endswith("abort.json"):
            return ok("", 0 if s["abort"] else 1)
        if command.startswith("test -e"):
            return ok("", 1)
        return ok()

    def helper(self, *arguments, timeout=90):
        return self.run("python3 -I ~/x/pi_night_host.py " + " ".join(arguments))

    def push(self, source, destination):
        pass

    def pull(self, source, destination):
        Path(destination).write_text("{}")


class FakeVm:
    def __init__(self):
        self.fail, self.calls = set(), []

    def call(self, *args, timeout=None, abort_check=None):
        self.calls.append(args)
        if args[0] == "pull":
            summary = (
                "fail"
                if "install" in self.fail and "install-result.json" in args[2]
                else "pass"
            )
            Path(args[3]).write_text(json.dumps({"summary": summary}))
        if (
            args[0] == "exec"
            and "install_guest.py" in args[2]
            and "install" in self.fail
        ):
            return 1, "failed", False
        return 0, "ok", False


def make_night(directory, suites=("install",), **state):
    directory = Path(directory)
    args = SimpleNamespace(
        run_dir=directory / "run",
        candidate=directory / "cand",
        image=directory / "img",
        suites=list(suites),
        interval=0,
    )
    args.run_dir.mkdir()
    host, vm = FakeHost(**state), FakeVm()
    deadline = datetime.now(timezone.utc) + timedelta(hours=6)
    return (
        night.Night(
            args,
            host,
            vm,
            deadline,
            {"end": deadline + timedelta(hours=1)},
            sleep=lambda s: None,
        ),
        host,
        vm,
    )


class NightTests(unittest.TestCase):
    def test_clean_run_purges_exactly_what_it_added(self):
        with tempfile.TemporaryDirectory() as d:
            n, host, vm = make_night(d)
            self.assertEqual(n.execute(), 0, n.verdict)
            self.assertEqual(
                n.verdict["added_packages"], ["libfoo", "qemu-system-arm", "qemu-utils"]
            )
            purge = [c for c in host.commands if "apt-get purge -y" in c]
            self.assertEqual(len(purge), 1)
            self.assertTrue(purge[0].endswith("libfoo qemu-system-arm qemu-utils"))
            self.assertEqual(host.state["packages"], {"bash": "5", "curl": "8"})
            self.assertEqual(
                [c[0] for c in vm.calls if c[0] in ("boot", "stop")], ["boot", "stop"]
            )
            self.assertTrue(
                all(
                    "--no-install-recommends" in c
                    for c in host.commands
                    if "apt-get install" in c or "apt-get -s install" in c
                )
            )
            self.assertFalse(
                any("docker" in c or "/opt/stacks" in c for c in host.commands)
            )

    def test_one_guest_at_a_time_with_a_guard_before_each(self):
        with tempfile.TemporaryDirectory() as d:
            n, host, vm = make_night(d, suites=("install", "container"))
            self.assertEqual(n.execute(), 0, n.verdict)
            self.assertEqual(
                [c[0] for c in vm.calls if c[0] in ("boot", "stop")],
                ["boot", "stop", "boot", "stop"],
            )
            self.assertGreaterEqual(
                len([c for c in host.commands if "guard --phase start" in c]), 3
            )

    def test_stopped_container_refuses_before_any_change(self):
        with tempfile.TemporaryDirectory() as d:
            n, host, vm = make_night(d)
            host.state["containers"] = STOPPED
            self.assertEqual(n.execute(), 3)
            self.assertFalse(
                any("apt-get" in c and "apt-get -s" not in c for c in host.commands)
            )
            self.assertEqual(vm.calls, [])
            self.assertTrue(n.verdict["refusals"])

    def test_low_memory_refuses_to_start(self):
        with tempfile.TemporaryDirectory() as d:
            n, host, vm = make_night(d)
            host.state["available"] = 2 * GIB
            self.assertEqual(n.execute(), 3)
            self.assertEqual(vm.calls, [])

    def test_watchdog_abort_stops_the_run_and_fails_it(self):
        with tempfile.TemporaryDirectory() as d:
            n, host, vm = make_night(d, suites=("install", "container"))
            original = vm.call

            def aborting(*args, **kwargs):
                if args[0] == "exec" and "install_guest.py" in args[2]:
                    host.state["abort"] = True
                return original(*args, **kwargs)

            vm.call = aborting
            self.assertEqual(n.execute(), 4)
            self.assertEqual([c[0] for c in vm.calls].count("boot"), 1)
            self.assertTrue(any("aborted" in p for p in n.verdict["problems"]))
            # Teardown still purged what was added and stopped the guest.
            self.assertEqual(host.state["packages"], {"bash": "5", "curl": "8"})
            self.assertIn("stop", [c[0] for c in vm.calls])

    def test_a_container_that_stopped_during_the_night_fails_the_diff(self):
        with tempfile.TemporaryDirectory() as d:
            n, host, vm = make_night(d)
            original = vm.call

            def stopping(*args, **kwargs):
                if args[0] == "stop":
                    host.state["containers"] = STOPPED
                return original(*args, **kwargs)

            vm.call = stopping
            self.assertEqual(n.execute(), 4)
            self.assertTrue(any(p.startswith("docker:") for p in n.verdict["problems"]))

    def test_suite_failure_is_exit_one_with_a_clean_host(self):
        with tempfile.TemporaryDirectory() as d:
            n, host, vm = make_night(d)
            vm.fail = {"install"}
            self.assertEqual(n.execute(), 1, n.verdict)
            self.assertEqual(n.verdict["problems"], [])


class WatchdogTests(unittest.TestCase):
    def run_watchdog(self, directory, containers, available_kib=6 * GIB):
        base = Path(directory)
        snap = {
            "containers": {"stdout": ROWS},
            "boot_id": "boot",
            "vmstat": {"oom_kill": 0},
            "captured_epoch": 0,
        }
        (base / "baseline.json").write_text(json.dumps(snap))
        args = SimpleNamespace(
            baseline=str(base / "baseline.json"),
            log=str(base / "w.jsonl"),
            abort_flag=str(base / "abort.json"),
            done_flag=str(base / "done"),
            deadline=1e12,
            interval=0,
            heartbeat=0,
        )
        meminfo = f"MemAvailable: {available_kib} kB\nSwapTotal: 2097152 kB\nSwapFree: 2097152 kB\n"
        reads = {
            "/proc/meminfo": meminfo,
            "/proc/sys/kernel/random/boot_id": "boot\n",
            "/proc/vmstat": "oom_kill 0\n",
            "/proc/loadavg": "0 0 0 1/1 1",
        }
        with (
            patch.object(
                ph,
                "containers",
                return_value={"exit": 0, "stdout": containers, "stderr": ""},
            ),
            patch.object(ph, "read", side_effect=lambda p: reads.get(str(p), "")),
            patch.object(ph, "attribution") as attribution,
            patch.object(ph, "stop_guests") as stop,
            patch.object(
                ph.time, "sleep", side_effect=lambda s: (base / "done").write_text("")
            ),
            patch.object(
                ph.subprocess, "run", side_effect=AssertionError("no command may run")
            ),
        ):
            return ph.watchdog(args), attribution, stop, base

    def test_quiet_run_exits_on_the_done_flag(self):
        with tempfile.TemporaryDirectory() as d:
            code, attribution, stop, base = self.run_watchdog(d, ROWS)
            self.assertEqual(code, 0)
            attribution.assert_not_called()
            self.assertFalse((base / "abort.json").exists())

    def test_container_stop_aborts_records_and_stops_only_the_guest(self):
        with tempfile.TemporaryDirectory() as d:
            code, attribution, stop, base = self.run_watchdog(d, STOPPED)
            self.assertEqual(code, 4)
            attribution.assert_called_once()
            stop.assert_called_once()
            self.assertIn("mattermost: state exited", (base / "abort.json").read_text())
            events = [
                json.loads(line) for line in (base / "w.jsonl").read_text().splitlines()
            ]
            self.assertIn("abort", [e["event"] for e in events])
            self.assertTrue(all("epoch" in e and "utc" in e for e in events))

    def test_memory_floor_aborts(self):
        with tempfile.TemporaryDirectory() as d:
            code, *_ = self.run_watchdog(d, ROWS, available_kib=GIB)
            self.assertEqual(code, 4)

    def test_watchdog_source_never_restarts_containers(self):
        import re

        source = Path(ph.__file__).read_text()
        for verb in ("start", "restart", "unpause", "rm", "kill", "compose", "stop"):
            self.assertIsNone(re.search(rf'"docker",\s*"{verb}"', source))


if __name__ == "__main__":
    unittest.main()
