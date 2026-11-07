#!/usr/bin/env python3
"""Fast self-tests for the rehearsal's ssh/scp stand-ins and stub host.

    cd tests/qualification/arm64 && python3 -m unittest discover -s rehearsal

These prove the stand-ins keep the ssh behaviours that matter (one command string,
call returns only after pipe EOF, stdin, exit status, refusal of real host names) and
that the stub docker/dpkg/apt answer in wybie's real formats. The slow end-to-end
scenarios are in scenarios.py.
"""

import json
import os
import signal
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(HERE.parent))
import transport  # noqa: E402
import wybie  # noqa: E402

ARM64 = HERE.parent


class Base(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.addCleanup(self.dir.cleanup)
        self.host = wybie.Wybie(Path(self.dir.name) / "w")
        self.addCleanup(self.host.release)
        self.addCleanup(self.host.kill_all)
        self.env = {
            "PATH": f"{HERE / 'bin'}:/usr/bin:/bin",
            "LIMEOS_REHEARSAL_HOST": wybie.HOST,
            "LIMEOS_REHEARSAL_HOME": str(self.host.home),
            "LIMEOS_REHEARSAL_CTL": str(self.host.ctl),
            "LIMEOS_REHEARSAL_TOKEN": self.host.token,
        }

    def ssh(self, command, host=wybie.HOST, timeout=20, **kwargs):
        return subprocess.run(
            ["ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=10", host, command],
            env=self.env,
            capture_output=True,
            text=True,
            timeout=timeout,
            **kwargs,
        )


class SshSemantics(Base):
    def test_one_command_string_status_streams_and_home(self):
        done = self.ssh("echo out; echo err >&2; echo $HOME; exit 7")
        self.assertEqual(done.returncode, 7)
        self.assertEqual(done.stdout.split(), ["out", str(self.host.home)])
        self.assertEqual(done.stderr.strip(), "err")

    def test_arguments_are_joined_like_ssh(self):
        done = subprocess.run(
            ["ssh", "-o", "BatchMode=yes", wybie.HOST, "echo", "a", "&&", "echo", "b"],
            env=self.env,
            capture_output=True,
            text=True,
            timeout=20,
        )
        self.assertEqual(done.stdout.split(), ["a", "b"])

    def test_stdin_is_forwarded(self):
        done = self.ssh("python3 -I - snapshot | head -c 20", input="print('from stdin')")
        self.assertEqual(done.returncode, 0)
        done = self.ssh("cat", input="hello")
        self.assertEqual(done.stdout, "hello")

    def test_a_backgrounded_job_holding_the_pipes_hangs_the_call(self):
        """The 2026-11-07 bug: ssh does not return while a background job holds stdout."""
        with self.assertRaises(subprocess.TimeoutExpired):
            self.ssh("sleep 30 & echo started", timeout=3)

    def test_a_detached_daemon_returns_at_once(self):
        began = time.monotonic()
        done = self.ssh("{ setsid nohup sleep 30 </dev/null >/dev/null 2>&1 & echo $!; }")
        self.assertLess(time.monotonic() - began, 5)
        os.kill(int(done.stdout.strip()), signal.SIGKILL)

    def test_the_callers_timeout_leaves_the_remote_job_running(self):
        marker = Path(self.dir.name) / "alive"
        with self.assertRaises(subprocess.TimeoutExpired):
            self.ssh(f"sleep 2; touch {marker}; sleep 30", timeout=1)
        time.sleep(3)
        self.assertTrue(marker.exists(), "sshd does not kill a job when the client dies")

    def test_real_and_unknown_hosts_are_refused_without_running_anything(self):
        marker = Path(self.dir.name) / "ran"
        for host in ("holly@wybie", "wybie", "someone@elsewhere"):
            done = self.ssh(f"touch {marker}", host=host)
            self.assertEqual(done.returncode, 255, host)
            self.assertIn("Could not resolve", done.stderr)
        self.assertFalse(marker.exists())

    def test_a_production_name_is_never_served_even_if_configured(self):
        self.env["LIMEOS_REHEARSAL_HOST"] = "holly@wybie"
        done = self.ssh("true", host="holly@wybie")
        self.assertEqual(done.returncode, 255)
        self.assertIn("never serves a production", done.stderr)

    def test_faults_are_applied_once_and_logged(self):
        self.host.set_faults([{"match": "echo ping", "action": "fail", "times": 1}])
        self.assertEqual(self.ssh("echo ping").returncode, 255)
        self.assertEqual(self.ssh("echo ping").stdout.strip(), "ping")
        self.assertEqual(len(self.host.transport()), 2)

    def test_scp_expands_home_both_ways_and_refuses_other_hosts(self):
        source = Path(self.dir.name) / "a.txt"
        source.write_text("data")
        scp = ["scp", "-o", "BatchMode=yes"]
        done = subprocess.run([*scp, str(source), f"{wybie.HOST}:limeos-arm64-qual/a.txt"], env=self.env, capture_output=True, text=True)
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertEqual((self.host.home / "limeos-arm64-qual/a.txt").read_text(), "data")
        back = Path(self.dir.name) / "b.txt"
        done = subprocess.run([*scp, f"{wybie.HOST}:~/limeos-arm64-qual/a.txt", str(back)], env=self.env, capture_output=True, text=True)
        self.assertEqual(back.read_text(), "data")
        done = subprocess.run([*scp, str(source), "holly@wybie:x"], env=self.env, capture_output=True, text=True)
        self.assertEqual(done.returncode, 255)

    def test_path_mapping_is_limited_to_the_files_the_harness_names(self):
        ctl = Path("/c")
        mapped = transport.map_paths(
            "test -c /dev/kvm; cat /proc/meminfo /etc/passwd; ls /usr/share/AAVMF/AAVMF_CODE.fd /usr/share/doc",
            transport.HOST_MAP,
            ctl=ctl,
        )
        self.assertEqual(
            mapped,
            "test -c /dev/null; cat /c/fake/proc/meminfo /etc/passwd; "
            "ls /c/usr/share/AAVMF/AAVMF_CODE.fd /usr/share/doc",
        )


class StubFormats(Base):
    def test_docker_formats_match_wybie_and_unknown_fields_fail(self):
        real = json.loads(wybie.FIXTURE.read_text())
        listed = self.ssh("docker ps -aq --no-trunc").stdout.split()
        self.assertEqual(len(listed), 18)
        inspect = self.ssh(
            "docker inspect --format '{{.Id}}|{{.Name}}|{{.State.Status}}|{{.State.StartedAt}}|{{.RestartCount}}' "
            + " ".join(listed)
        )
        self.assertEqual(
            sorted(inspect.stdout.splitlines()), sorted(real["containers"]["stdout"].splitlines())
        )
        ps = self.ssh(
            "docker ps -a --no-trunc --format '{{.ID}}\\t{{.Names}}\\t{{.State}}\\t{{.RunningFor}}\\t{{.Status}}'"
        )
        self.assertEqual(sorted(ps.stdout.splitlines()), sorted(real["docker_ps"]["stdout"].splitlines()))
        bad = self.ssh("docker ps --format '{{.Bogus}}'")
        self.assertNotEqual(bad.returncode, 0)

    def test_dpkg_and_apt_mark_reproduce_the_real_lists(self):
        real = json.loads(wybie.FIXTURE.read_text())
        done = self.ssh("dpkg-query -W -f '${Package}:${Architecture}\\t${Version}\\n'")
        self.assertEqual(done.stdout, real["packages"]["stdout"])
        self.assertEqual(self.ssh("apt-mark showmanual").stdout, real["manual_packages"]["stdout"])

    def test_apt_requires_noninteractive_flags_and_root_like_sudo(self):
        no_sudo = self.ssh("sudo apt-get -s install --no-install-recommends qemu-utils")
        self.assertNotEqual(no_sudo.returncode, 0)
        no_flag = self.ssh("sudo -n apt-get -s install qemu-utils")
        self.assertNotEqual(no_flag.returncode, 0)

    def test_helper_snapshot_runs_against_the_stub_host(self):
        helper = (ARM64 / "pi_night_host.py").read_text()
        done = self.ssh("python3 -I - snapshot", input=helper)
        self.assertEqual(done.returncode, 0, done.stderr)
        snap = json.loads(done.stdout)
        self.assertEqual(snap["meminfo"]["MemAvailable"], 6064112)
        self.assertEqual(snap["containers"]["stdout"].count("\n"), 18)
        self.assertEqual(snap["qemu_processes"], [])
        self.assertEqual(snap["boot_id"], "2c58cdca-4786-4e0e-a053-4907aa7b36b5")


if __name__ == "__main__":
    unittest.main()
