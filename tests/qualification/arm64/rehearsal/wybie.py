#!/usr/bin/env python3
"""A throw-away imitation of wybie plus a driver for the real Pi night orchestrator.

`Rehearsal` builds a temporary "wybie" (home directory, stub commands, state seeded
from the real 2026-11-07 snapshot), runs `launch.py --execute` (the real
run_pi_night.py and native_vm.py through the ssh/scp stand-ins in bin/), then reports
what happened and what changed on the host. Nothing is contacted; no real sudo, apt,
docker or qemu is used.
"""

import fcntl
import hashlib
import json
import os
import secrets
import shutil
import signal
import subprocess
import sys
import tarfile
import tempfile
import time
from datetime import UTC, datetime, timedelta
from pathlib import Path

HERE = Path(__file__).resolve().parent
ARM64 = HERE.parent
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(ARM64))
import remote_stubs  # noqa: E402

HOST = "rehearsal@wybie-rehearsal"
FIXTURE = HERE / "fixtures/wybie-host-pre.json"
PYTHON_LAUNCHER = """#!{python}
import sys
sys.path.insert(0, {here!r})
import remote_stubs
sys.exit(remote_stubs.main({name!r}, {ctl!r}))
"""
QEMU_LAUNCHER = """#!/bin/bash
exec -a "$0" {python} {here}/remote_stubs.py qemu-system-aarch64 {ctl} "$@"
"""
PYTHON3_LAUNCHER = """#!/bin/bash
[ "$1" = "-I" ] && shift
export LIMEOS_REHEARSAL_CTL={ctl}
exec {python} -I {here}/supervisor_runner.py "$@"
"""
HOST_COMMANDS = (
    "docker dpkg-query apt-mark apt-get uname free getconf systemctl ss journalctl "
    "sudo install qemu-img"
).split()
GUEST_COMMANDS = {
    "python3": "guest-python3",
    "journalctl": "guest-journalctl",
    "systemctl": "guest-systemctl",
}


def utc(moment):
    return moment.strftime("%Y-%m-%dT%H:%M:%SZ")


class Wybie:
    """The fake host: files only. Mutating helpers act like outside events."""

    def __init__(self, root, qemu="installed"):
        self.root = Path(root)
        self.home = self.root / "home/holly"
        self.ctl = self.root / "ctl"
        self.token = secrets.token_hex(8)
        # Rehearsals share /tmp names and sweep processes by pattern: run one at a time.
        self.lock = open("/tmp/limeos-rehearsal.lock", "a+")
        fcntl.flock(self.lock, fcntl.LOCK_EX)
        self.build(qemu)

    # -- construction
    def build(self, qemu):
        real = json.loads(FIXTURE.read_text())
        for sub in ("bin", "guest-bin", "state", "fake", "guests", "markers", "usr/share/AAVMF"):
            (self.ctl / sub).mkdir(parents=True, exist_ok=True)
        staging = self.home / "limeos-arm64-qual"
        (staging / "image").mkdir(parents=True)
        (staging / "runs").mkdir()
        for name in ("preparation-after.json", "preparation-before.json", "preparation-install.txt"):
            (staging / name).write_text("{}\n")
        (self.ctl / "usr/share/AAVMF/AAVMF_CODE.fd").write_bytes(b"code")
        (self.ctl / "usr/share/AAVMF/AAVMF_VARS.fd").write_bytes(b"vars")
        rows = {}
        for line in real["docker_ps"]["stdout"].splitlines():
            ident, name, status, running_for, text = line.split("\t")
            rows[ident] = dict(
                id=ident, name=name, state=status, running_for=running_for, status=text
            )
        for line in real["containers"]["stdout"].splitlines():
            ident, _, _, started, restarts = line.split("|")
            rows[ident].update(started_at=started, restarts=int(restarts))
        remote_stubs.CTL = self.ctl
        write = lambda name, text: (self.ctl / "state" / name).write_text(text)
        write("containers.json", json.dumps(list(rows.values()), indent=1))
        packages = real["packages"]["stdout"]
        manual = real["manual_packages"]["stdout"]
        if qemu != "installed":
            gone = {c["name"] for c in remote_stubs.catalog()}
            if qemu == "partial":
                gone = {"qemu-utils"}
            packages = "".join(
                f"{l}\n" for l in packages.splitlines() if l.split(":")[0] not in gone
            )
            drop = set(remote_stubs.QEMU_REQUESTED) if qemu == "absent" else {"qemu-utils"}
            manual = "".join(f"{l}\n" for l in manual.splitlines() if l not in drop)
        write("packages.tsv", packages)
        write("manual.txt", manual)
        write("units.txt", real["running_units"]["stdout"])
        write("listeners.txt", real["listeners"]["stdout"])
        write("uname.txt", real["uname"] + "\n")
        write("free.txt", real["free_m"])
        mem = dict(real["meminfo"])
        proc = {
            "meminfo": mem,
            "loadavg": real["loadavg"],
            "vmstat": real["vmstat"],
            "boot_id": real["boot_id"],
            "temperature": real["temperature_millidegrees"],
        }
        write("proc.json", json.dumps(proc, indent=1))
        remote_stubs.render_proc(self.ctl)
        python = sys.executable
        for name in HOST_COMMANDS:
            self.launcher(self.ctl / "bin" / name, PYTHON_LAUNCHER, name=name)
        for name, stub in GUEST_COMMANDS.items():
            self.launcher(self.ctl / "guest-bin" / name, PYTHON_LAUNCHER, name=stub)
        self.launcher(self.ctl / "bin/qemu-system-aarch64", QEMU_LAUNCHER)
        self.launcher(self.ctl / "bin/rehearsal-python3", PYTHON3_LAUNCHER)
        self.set_scenario({})
        self.set_faults([])
        self.initial = self.digest()

    def launcher(self, path, template, **extra):
        path.write_text(
            template.format(python=sys.executable, here=str(HERE), ctl=str(self.ctl), **extra)
        )
        path.chmod(0o755)

    def set_scenario(self, value):
        (self.ctl / "scenario.json").write_text(json.dumps(value))

    def set_faults(self, faults):
        (self.ctl / "faults.json").write_text(json.dumps(faults))

    # -- observation
    def containers(self):
        return json.loads((self.ctl / "state/containers.json").read_text())

    def digest(self):
        """Everything the night promises to leave exactly as found."""
        staging = self.home / "limeos-arm64-qual"
        listing = sorted(
            str(p.relative_to(self.home))
            for p in staging.rglob("*")
        ) if staging.exists() else None
        return {
            "packages": (self.ctl / "state/packages.tsv").read_text(),
            "manual": (self.ctl / "state/manual.txt").read_text(),
            "containers": [
                (c["name"], c["state"], c["started_at"], c["restarts"]) for c in self.containers()
            ],
            "units": (self.ctl / "state/units.txt").read_text(),
            "listeners": (self.ctl / "state/listeners.txt").read_text(),
            "staging": listing,
            "home": sorted(p.name for p in self.home.iterdir()),
            "boot_id": json.loads((self.ctl / "state/proc.json").read_text())["boot_id"],
        }

    def calls(self, name=None):
        path = self.ctl / "calls.jsonl"
        rows = [json.loads(l) for l in path.read_text().splitlines()] if path.exists() else []
        return [r for r in rows if name in (None, r["cmd"])]

    def qemu_launches(self):
        path = self.ctl / "qemu-launches.jsonl"
        return [json.loads(l) for l in path.read_text().splitlines()] if path.exists() else []

    def transport(self):
        path = self.ctl / "transport.jsonl"
        return [json.loads(l) for l in path.read_text().splitlines()] if path.exists() else []

    # -- processes
    def process_ids(self):
        """Processes belonging to this rehearsal: token in the environment, or cwd/argv under it."""
        mine = []
        for entry in Path("/proc").iterdir():
            if not entry.name.isdigit() or int(entry.name) == os.getpid():
                continue
            try:
                environ = (entry / "environ").read_bytes()
                cmdline = (entry / "cmdline").read_bytes().decode(errors="replace")
                cwd = os.readlink(entry / "cwd")
                stat = (entry / "stat").read_text()
            except OSError:
                continue
            if stat[stat.rindex(")") + 2 :].split()[0] == "Z":
                continue
            if (
                f"LIMEOS_REHEARSAL_TOKEN={self.token}".encode() in environ
                or str(self.ctl) in cmdline
                or cwd.startswith(str(self.root))
                or ("guest_supervisor.py" in cmdline and "limeos-arm64-night-" in cmdline)
            ):
                mine.append(int(entry.name))
        return mine

    def kill_all(self):
        for _ in range(3):
            pids = self.process_ids()
            if not pids:
                return
            for pid in pids:
                try:
                    os.kill(pid, signal.SIGKILL)
                except OSError:
                    pass
            time.sleep(0.3)

    def release(self):
        self.lock.close()

    # -- outside events
    def stop_container(self, name):
        return remote_stubs.stop_container(name)

    def change_proc(self, **changes):
        remote_stubs.change_proc(self.ctl, **changes)


class Candidate:
    """Synthetic stand-in for the prepared candidate and the Debian image."""

    def __init__(self, root):
        self.root = Path(root)
        inputs = self.root / "candidate/inputs"
        (inputs / "repo").mkdir(parents=True)
        (inputs / "repo/Release").write_text("rehearsal\n")
        fixtures = self.root / "payload"
        (fixtures / "source/tests/fixtures").mkdir(parents=True)
        (fixtures / "source/tests/fixtures/werkzeug-hashes.json").write_text("{}\n")
        with tarfile.open(inputs / "source.tar.gz", "w:gz") as archive:
            archive.add(fixtures / "source", arcname="source")
        (inputs / "source-manifest.json").write_text("{}\n")
        (self.root / "candidate/build-result.json").write_text(
            json.dumps(
                {"identity": {"package_version": "0.0.0+rehearsal", "source_commit": "0" * 40}}
            )
        )
        self.path = self.root / "candidate"
        self.image = self.root / "debian-12-genericcloud-arm64.qcow2"
        self.image.write_bytes(b"QFI\xfb" + os.urandom(4096))


class Result:
    def __init__(self, code, stdout, stderr, run_dir):
        self.code, self.stdout, self.stderr, self.run_dir = code, stdout, stderr, run_dir

    def verdict(self):
        return json.loads((self.run_dir / "verdict.json").read_text())

    def host_commands(self):
        path = self.run_dir / "host-commands.jsonl"
        return [json.loads(l) for l in path.read_text().splitlines()] if path.exists() else []

    def watchdog_events(self):
        path = self.run_dir / "watchdog.jsonl"
        return [json.loads(l) for l in path.read_text().splitlines()] if path.exists() else []


class Rehearsal:
    """One scratch host + candidate + run directory; use as a context manager."""

    def __init__(self, qemu="installed", keep=False):
        self.keep = keep
        self.root = Path(tempfile.mkdtemp(prefix="limeos-rehearsal-"))
        self.wybie = Wybie(self.root / "wybie", qemu=qemu)
        self.candidate = Candidate(self.root / "inputs")
        self.state = self.root / "state"
        self.state.mkdir()
        self.run_dir = self.root / "run"
        self.tmp_before = set(Path("/tmp").glob("limeos-arm64-*"))
        now = datetime.now(UTC)
        self.deadline = now + timedelta(hours=10)
        self.authorization = self.root / "authorization.json"
        self.authorization.write_text(
            json.dumps(
                {
                    "version": 1,
                    "host": HOST,
                    "not_before": utc(now - timedelta(minutes=10)),
                    "not_after": utc(now + timedelta(hours=11)),
                    "authorized_by": "rehearsal",
                    "reference": "local rehearsal, nothing real",
                    "scope": "rehearsal host only",
                }
            )
        )
        self.children = []
        self.timeout_scale = "1"

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        self.close()

    def close(self):
        for child in self.children:
            if child.poll() is None:
                try:
                    os.killpg(child.pid, signal.SIGKILL)
                except OSError:
                    pass
        self.wybie.kill_all()
        for path in set(Path("/tmp").glob("limeos-arm64-*")) - self.tmp_before:
            subprocess.run(["rm", "-rf", "--", str(path)], check=False)
        if not self.keep:
            shutil.rmtree(self.root, ignore_errors=True)
        self.wybie.release()

    def environment(self):
        env = {
            k: v for k, v in os.environ.items() if not k.startswith(("LIMEOS_", "SSH_"))
        }
        env.update(
            PATH=f"{HERE / 'bin'}:{env.get('PATH', '/usr/bin:/bin')}",
            LIMEOS_REHEARSAL_HOST=HOST,
            LIMEOS_REHEARSAL_HOME=str(self.wybie.home),
            LIMEOS_REHEARSAL_CTL=str(self.wybie.ctl),
            LIMEOS_REHEARSAL_TOKEN=self.wybie.token,
            LIMEOS_REHEARSAL_IMAGE=str(self.candidate.image),
            LIMEOS_REHEARSAL_TIMEOUT_SCALE=self.timeout_scale,
            LIMEOS_QUAL_STATE_ROOT=str(self.state),
        )
        return env

    def command(self, *extra, teardown_only=False):
        return [
            sys.executable,
            str(HERE / "launch.py"),
            "--host",
            HOST,
            "--authorization",
            str(self.authorization),
            "--deadline",
            utc(self.deadline),
            "--candidate",
            str(self.candidate.path),
            "--run-dir",
            str(self.run_dir),
            "--image",
            str(self.candidate.image),
            "--interval",
            "5",
            "--execute",
            *(["--teardown-only"] if teardown_only else []),
            *extra,
        ]

    def start(self, *extra, teardown_only=False, env=None):
        child = subprocess.Popen(
            self.command(*extra, teardown_only=teardown_only),
            env={**self.environment(), **(env or {})},
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            start_new_session=True,
        )
        self.children.append(child)
        return child

    def finish(self, child, timeout=900):
        try:
            out, err = child.communicate(timeout=timeout)
        except subprocess.TimeoutExpired:
            os.killpg(child.pid, signal.SIGKILL)
            out, err = child.communicate()
            err += "\nREHEARSAL: orchestrator exceeded its time limit"
        return Result(child.returncode, out, err, self.run_dir)

    def run(self, *extra, timeout=900, env=None):
        return self.finish(self.start(*extra, env=env), timeout)

    def teardown_only(self, timeout=600):
        return self.finish(self.start(teardown_only=True), timeout)

    def wait_for(self, predicate, seconds=120, what="condition"):
        end = time.monotonic() + seconds
        while time.monotonic() < end:
            if predicate():
                return
            time.sleep(0.2)
        raise TimeoutError(f"rehearsal: timed out waiting for {what}")
