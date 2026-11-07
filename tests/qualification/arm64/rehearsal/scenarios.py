#!/usr/bin/env python3
"""End-to-end rehearsal of the Pi night run: the real orchestrator through a real shell.

    cd tests/qualification/arm64 && python3 rehearsal/scenarios.py -v

Each scenario builds a fresh imitation of wybie (real 2026-11-07 snapshot: 14 running
and 4 exited containers, 758 packages, qemu already installed), runs
`launch.py --execute` for the real run_pi_night.py/native_vm.py/pi_night_host.py/
guest_supervisor.py with ssh and scp replaced by local shells, and asserts on the
verdict, exit code and the state of the imitation host afterwards. A full pass takes
several minutes (the orchestrator itself sleeps ~12 s to let the watchdog settle).
Select one with e.g. `python3 rehearsal/scenarios.py Night.test_happy_path_qemu_already_installed`.
"""

import json
import os
import signal
import sys
import time
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(HERE.parent))
import wybie  # noqa: E402

SUITES = ["install", "container", "stack", "scale"]
SUDO_ALLOWED = {"apt-get", "/usr/bin/mktemp", "/usr/bin/install", "/usr/bin/sha256sum",
                "/usr/bin/cat", "/usr/bin/rm", "/usr/bin/env"}


class Night(unittest.TestCase):
    def rehearse(self, qemu="installed", scenario=None, faults=None):
        rehearsal = wybie.Rehearsal(qemu=qemu)
        self.addCleanup(rehearsal.close)
        rehearsal.wybie.set_scenario(scenario or {})
        rehearsal.wybie.set_faults(faults or [])
        return rehearsal

    # -- shared assertions
    def assertHostIdentical(self, rehearsal, ignore=()):
        after = rehearsal.wybie.digest()
        for key, value in rehearsal.wybie.initial.items():
            if key not in ignore:
                self.assertEqual(after[key], value, f"host {key} differs after the run")
        self.assertEqual(rehearsal.wybie.process_ids(), [], "processes left behind")
        leftovers = set(Path("/tmp").glob("limeos-arm64-*")) - rehearsal.tmp_before
        self.assertEqual(leftovers, set(), "temporary files left in /tmp")

    def assertReadOnlyDocker(self, rehearsal):
        verbs = {c["argv"][0] for c in rehearsal.wybie.calls("docker") if c["argv"]}
        self.assertLessEqual(verbs, {"ps", "inspect", "events"}, "the tool must only read docker state")

    def assertSudoScoped(self, rehearsal):
        for call in rehearsal.wybie.calls("sudo"):
            args = [a for a in call["argv"] if a != "-n" and "=" not in a.split("/")[0]]
            self.assertIn(args[0], SUDO_ALLOWED, f"unexpected sudo command {args[:3]}")

    def assertCleanOutput(self, result):
        self.assertNotIn("Traceback", result.stderr, result.stderr[-2000:])

    # -- scenarios
    def test_happy_path_qemu_already_installed(self):
        """wybie as found on 2026-11-07: 4 containers already stopped, qemu from 12 October present."""
        r = self.rehearse()
        running = [c for c in r.wybie.containers() if c["state"] == "running"]
        self.assertEqual((len(running), len(r.wybie.containers()) - len(running)), (14, 4))
        result = r.run()
        self.assertEqual(result.code, 0, result.stdout[-1500:] + result.stderr[-3000:])
        self.assertCleanOutput(result)
        verdict = result.verdict()
        self.assertEqual(verdict["refusals"], [])
        self.assertEqual(verdict["added_packages"], [])
        self.assertEqual(verdict["problems"], [])
        self.assertEqual({s: v["summary"] for s, v in verdict["suites"].items()}, dict.fromkeys(SUITES, "pass"))
        # nothing was installed or purged: only the two simulations ran
        apt = [c["argv"] for c in r.wybie.calls("apt-get")]
        self.assertTrue(apt and all("-s" in a for a in apt), apt)
        # four guests, one at a time, inside the agreed envelope
        launches = r.wybie.qemu_launches()
        self.assertEqual([l["port"] for l in launches], [22931, 22932, 22933, 22934])
        self.assertEqual({(l["memory"], l["cpus"]) for l in launches}, {(1536, 2)})
        self.assertHostIdentical(r)
        self.assertReadOnlyDocker(r)
        self.assertSudoScoped(r)
        events = [e["event"] for e in result.watchdog_events()]
        self.assertIn("watchdog_started", events)
        self.assertIn("done_flag", events)
        self.assertNotIn("abort", events)
        # the watchdog launch must return at once (2026-11-07 regression)
        commands = result.host_commands()
        launch = next(i for i, c in enumerate(commands) if "setsid nohup" in c["command"])
        began = lambda c: time.mktime(time.strptime(c["started"], "%Y-%m-%dT%H:%M:%SZ"))
        self.assertLess(began(commands[launch + 1]) - began(commands[launch]), 20)

    def test_every_container_stopped_refuses_before_touching_anything(self):
        r = self.rehearse()
        for c in r.wybie.containers():
            r.wybie.stop_container(c["name"])
        r.wybie.initial = r.wybie.digest()
        result = r.run()
        self.assertEqual(result.code, 3, result.stderr[-2000:])
        self.assertCleanOutput(result)
        self.assertIn("no running containers", " ".join(result.verdict()["refusals"]))
        self.assertEqual(r.wybie.calls("apt-get"), [])
        self.assertEqual(r.wybie.qemu_launches(), [])
        self.assertHostIdentical(r)

    def test_docker_unreadable_refuses(self):
        r = self.rehearse(scenario={"docker_denied": True})
        result = r.run()
        self.assertEqual(result.code, 3, result.stderr[-2000:])
        self.assertIn("docker state unreadable", " ".join(result.verdict()["refusals"]))
        self.assertHostIdentical(r)

    def test_low_memory_refuses(self):
        r = self.rehearse()
        r.wybie.change_proc(MemAvailable=int(2.5 * 1024 * 1024))
        result = r.run()
        self.assertEqual(result.code, 3, result.stderr[-2000:])
        self.assertIn("MemAvailable", " ".join(result.verdict()["refusals"]))
        self.assertEqual(r.wybie.calls("apt-get"), [])
        self.assertEqual(r.wybie.qemu_launches(), [])
        self.assertHostIdentical(r)

    def test_busy_host_refuses(self):
        r = self.rehearse()
        r.wybie.change_proc(loadavg="3.10 2.90 2.00 4/900 1234")
        result = r.run()
        self.assertEqual(result.code, 3, result.stderr[-2000:])
        self.assertIn("load average", " ".join(result.verdict()["refusals"]))
        self.assertHostIdentical(r)

    def test_qemu_absent_is_installed_then_purged_exactly(self):
        r = self.rehearse(qemu="absent")
        result = r.run()
        self.assertEqual(result.code, 0, result.stdout[-1500:] + result.stderr[-3000:])
        verdict = result.verdict()
        self.assertEqual(len(verdict["added_packages"]), 41)
        self.assertIn("qemu-system-arm:arm64", verdict["added_packages"])
        self.assertEqual(verdict["problems"], [])
        self.assertNotIn("notes", verdict)
        apt = [c["argv"] for c in r.wybie.calls("apt-get")]
        self.assertTrue(any("install" in a and "-y" in a for a in apt))
        self.assertTrue(any("purge" in a and "-y" in a for a in apt))
        self.assertHostIdentical(r)
        self.assertSudoScoped(r)

    def test_partly_installed_qemu_adds_and_purges_only_the_gap(self):
        r = self.rehearse(qemu="partial")
        result = r.run("--suites", "install")
        self.assertEqual(result.code, 0, result.stdout[-1500:] + result.stderr[-3000:])
        self.assertEqual(result.verdict()["added_packages"], ["qemu-utils:arm64"])
        self.assertHostIdentical(r)

    def test_qemu_install_that_would_upgrade_something_refuses(self):
        r = self.rehearse(qemu="absent", scenario={"apt_upgrade": True})
        result = r.run()
        self.assertEqual(result.code, 3, result.stderr[-2000:])
        self.assertIn("upgrade or remove", result.verdict()["refused"])
        self.assertEqual([c for c in r.wybie.calls("apt-get") if "-y" in c["argv"]], [])
        self.assertHostIdentical(r)

    def test_failed_qemu_install_is_purged_again(self):
        r = self.rehearse(qemu="absent", scenario={"apt_install_fail": True})
        result = r.run()
        self.assertEqual(result.code, 3, result.stderr[-2000:])
        self.assertTrue(result.verdict()["added_packages"], "half-installed packages must be seen")
        self.assertHostIdentical(r)

    def test_purge_that_would_remove_more_is_not_run(self):
        r = self.rehearse(qemu="absent", scenario={"purge_collateral": "bash"})
        result = r.run("--suites", "install")
        self.assertEqual(result.code, 4, result.stderr[-2000:])
        self.assertIn("more than was added", " ".join(result.verdict()["problems"]))
        apt = [c["argv"] for c in r.wybie.calls("apt-get")]
        self.assertFalse([a for a in apt if "purge" in a and "-y" in a])
        self.assertEqual(r.wybie.process_ids(), [])

    def test_sudo_denied_refuses(self):
        r = self.rehearse(qemu="absent", scenario={"sudo_denied": True})
        result = r.run()
        self.assertEqual(result.code, 3, result.stderr[-2000:])
        self.assertIn("simulation failed", result.verdict()["refused"])
        self.assertHostIdentical(r)

    def test_a_failing_guest_suite_exits_one_with_the_host_identical(self):
        r = self.rehearse(scenario={"suites": {"container": {"exit": 1, "summary": "fail"}}})
        result = r.run()
        self.assertEqual(result.code, 1, result.stdout[-1500:] + result.stderr[-3000:])
        suites = {s: v["summary"] for s, v in result.verdict()["suites"].items()}
        self.assertEqual(suites, {"install": "pass", "container": "fail", "stack": "pass", "scale": "pass"})
        self.assertEqual(result.verdict()["problems"], [])
        self.assertHostIdentical(r)

    def test_watchdog_aborts_when_a_running_container_is_stopped(self):
        """An unidentified client stops jellyfin 3 s into the first suite."""
        r = self.rehearse(
            scenario={
                "suites": {
                    "install": {
                        "actions": [{"at": 3, "do": "docker_stop", "name": "jellyfin"}],
                        "sleep": 300,
                    }
                }
            }
        )
        result = r.run()
        self.assertEqual(result.code, 4, result.stdout[-1500:] + result.stderr[-3000:])
        self.assertCleanOutput(result)
        verdict = result.verdict()
        self.assertIn("jellyfin: state exited", verdict["watchdog_abort"])
        self.assertIn("watchdog aborted the run", " ".join(verdict["problems"]))
        events = {e["event"]: e for e in result.watchdog_events()}
        self.assertIn("abort", events)
        self.assertIn("jellyfin", events["attribution"]["docker_events"]["stdout"])
        # no second guest; containers left exactly as the stopper left them
        self.assertEqual(len(r.wybie.qemu_launches()), 1)
        states = {c["name"]: c["state"] for c in r.wybie.containers()}
        self.assertEqual(states["jellyfin"], "exited")
        self.assertEqual(sum(s == "running" for s in states.values()), 13)
        self.assertReadOnlyDocker(r)
        # teardown still ran: staging restored, nothing added or left behind
        self.assertHostIdentical(r, ignore=("containers",))
        self.assertTrue(any("docker:" in p for p in verdict["problems"]))

    def test_watchdog_aborts_on_memory_pressure_mid_run(self):
        r = self.rehearse(
            scenario={
                "suites": {
                    "install": {
                        "actions": [{"at": 3, "do": "meminfo", "set": {"MemAvailable": 1500 * 1024}}],
                        "sleep": 300,
                    }
                }
            }
        )
        result = r.run()
        self.assertEqual(result.code, 4, result.stdout[-1500:] + result.stderr[-3000:])
        self.assertIn("MemAvailable", result.verdict()["watchdog_abort"])
        self.assertEqual(len(r.wybie.qemu_launches()), 1)
        self.assertEqual(r.wybie.process_ids(), [])

    def test_a_service_that_stops_during_the_run_is_reported(self):
        r = self.rehearse()
        text = (r.wybie.ctl / "state/units.txt").read_text()
        trimmed = "".join(l + "\n" for l in text.splitlines() if not l.startswith("cron.service"))
        r.wybie.set_scenario({"suites": {"container": {"actions": [{"at": 1, "do": "units", "text": trimmed}]}}})
        result = r.run("--suites", "container")
        self.assertEqual(result.code, 4, result.stderr[-2000:])
        self.assertIn("running units differ", " ".join(result.verdict()["problems"]))

    # -- ssh transport failures: timeouts are compressed (x0.05, floor 3 s)
    def test_hung_install_simulation_times_out_and_tears_down(self):
        r = self.rehearse(faults=[{"match": "apt-get -s install", "action": "hang", "times": 1}])
        r.timeout_scale = "0.05"
        result = r.run()
        self.assertEqual(result.code, 3, result.stdout[-1500:] + result.stderr[-3000:])
        self.assertCleanOutput(result)
        self.assertIn("simulation failed", result.verdict()["refused"])
        timed_out = [c for c in result.host_commands() if c["exit"] == 124]
        self.assertTrue(timed_out and "timed out" in timed_out[0]["stderr"], timed_out)
        self.assertHostIdentical(r)

    def test_hung_purge_leaves_a_clear_problem_and_finishes_teardown(self):
        r = self.rehearse(qemu="absent", faults=[{"match": "apt-get -s purge", "action": "hang", "times": 1}])
        r.timeout_scale = "0.05"
        result = r.run("--suites", "install")
        self.assertEqual(result.code, 4, result.stdout[-1500:] + result.stderr[-3000:])
        self.assertCleanOutput(result)
        problems = " ".join(result.verdict()["problems"])
        self.assertIn("purge simulation failed", problems)
        self.assertIn("remove by hand", problems)
        after = r.wybie.digest()
        self.assertEqual(after["staging"], r.wybie.initial["staging"], "staging must still be cleaned")
        self.assertEqual(r.wybie.process_ids(), [], "watchdog and guests must be gone")
        self.assertNotEqual(after["packages"], r.wybie.initial["packages"])

    def test_pipe_holding_watchdog_launch_does_not_hang_the_night(self):
        """The 2026-11-07 failure class: the call returns only when a held pipe is released."""
        r = self.rehearse(faults=[{"match": "setsid nohup", "action": "hold", "times": 1}])
        r.timeout_scale = "0.05"
        result = r.run("--suites", "install")
        self.assertEqual(result.code, 0, result.stdout[-1500:] + result.stderr[-3000:])
        self.assertCleanOutput(result)
        launch = [c for c in result.host_commands() if "setsid nohup" in c["command"]]
        self.assertEqual([c["exit"] for c in launch], [124])
        # the job that held the pipes is still there, as it would be on a real host
        holders = [p for p in r.wybie.process_ids() if "sleep" in Path(f"/proc/{p}/cmdline").read_text()]
        for pid in holders:
            os.kill(pid, signal.SIGKILL)
        self.assertEqual(len(holders), 1)
        time.sleep(0.5)
        self.assertHostIdentical(r)

    def test_ssh_dropping_on_the_first_guard_refuses(self):
        r = self.rehearse(faults=[{"match": "guard --phase start", "action": "fail", "times": 1}])
        result = r.run()
        self.assertEqual(result.code, 3, result.stdout[-1500:] + result.stderr[-3000:])
        self.assertIn("guard refused", result.verdict()["refused"])
        self.assertHostIdentical(r)

    def test_hung_boot_leaves_no_guest_and_a_clean_host(self):
        r = self.rehearse(faults=[{"match": "mktemp", "action": "hang", "times": 1}])
        result = r.run("--suites", "install")
        self.assertEqual(result.code, 1, result.stdout[-1500:] + result.stderr[-3000:])
        self.assertCleanOutput(result)
        self.assertEqual(result.verdict()["suites"]["install"]["summary"], "boot failed")
        self.assertEqual(r.wybie.qemu_launches(), [])
        self.assertHostIdentical(r)

    def test_a_crash_mid_run_is_recovered_by_teardown_only(self):
        r = self.rehearse(qemu="absent", scenario={"suites": {"install": {"sleep": 600}}})
        child = r.start()
        r.wait_for(lambda: (r.wybie.ctl / "markers/install.started").exists(), 180, "the first suite")
        os.killpg(child.pid, signal.SIGKILL)
        child.communicate()
        # the host is now in the half-way state a dead workstation leaves behind
        self.assertTrue(r.wybie.process_ids(), "guest, supervisor and watchdog should survive")
        self.assertNotEqual(r.wybie.digest()["packages"], r.wybie.initial["packages"])
        result = r.teardown_only()
        self.assertEqual(result.code, 0, result.stdout[-1500:] + result.stderr[-3000:])
        self.assertCleanOutput(result)
        self.assertEqual(result.verdict()["problems"], [])
        self.assertEqual(len(result.verdict()["added_packages"]), 41)
        self.assertHostIdentical(r)


if __name__ == "__main__":
    unittest.main()
