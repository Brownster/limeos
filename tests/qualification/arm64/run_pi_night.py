#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""One-command overnight native ARM64 qualification of a cross-built candidate.

Default is `--check`: validate every local input and print the plan, with no SSH.
`--execute` runs the night against the KVM host. LimeOS is NEVER installed on the
host; guests are disposable QEMU/KVM VMs, one at a time. Nothing here touches
Docker, /opt/stacks or the frozen application: the host only runs read-only
snapshots, the watchdog and QEMU.

    run_pi_night.py --host holly@wybie --authorization AUTH.json \
        --deadline 2026-11-06T05:30:00Z --candidate OUT --run-dir RUN [--execute]

Sequence: pre-snapshot -> refusals (a container not running, qemu already
running, guard) -> QEMU install `--no-install-recommends` (exact additions
recorded) -> image staging -> detached docker/memory watchdog -> per suite:
guard, boot (<= 1536 MiB, 2 vCPU, nice 19, ionice idle, host-owned deadline
supervisor), run, collect, stop -> teardown that purges exactly the added
packages -> post-snapshot diff. Containers are never restarted by this tool.

Exit codes: 0 all suites passed and the host is identical afterwards; 1 a suite
failed (host identical); 2 usage or local input error; 3 refused to start;
4 watchdog abort, teardown problem or host state differs afterwards.
"""

import argparse
import hashlib
import json
import re
import shlex
import subprocess
import sys
import time
from datetime import UTC, datetime, timedelta
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import native_vm
import pi_night_host as ph

ROOT = HERE.parents[2]
IMAGE_SHA512 = "835b929522716de9798d1c0fbe61caae8313ab78858b278f3389a2db922ae42e4619b4a776ecad39583c149fb85334a2f8cdae685bc56987160e5c86a2354375"
DEFAULT_IMAGE = (
    ROOT / ".cache/arm64-qual/integrated-2026-10-12/debian-12-genericcloud-arm64.qcow2"
)
QEMU_PACKAGES = ("qemu-system-arm", "qemu-efi-aarch64", "qemu-utils")
CPUS, MEMORY_MIB, DISK = 2, 1536, "16G"
STAGING = (
    "limeos-arm64-qual"  # under the SSH user's home, same as native_vm.REMOTE_BASE
)
NIGHT = f"{STAGING}/night"
LEAD_MINUTES = 12  # a guest must be able to finish this long before the deadline
TEARDOWN_MINUTES = 20  # authorization must outlast the deadline by this much
GUEST = "/root/qual/fixtures/tests/qualification/arm64"
COMMON = "--repo /opt/limeos-repo --expected /root/qual/build-result.json --hashes /root/werkzeug-hashes.json"
SUITES = {
    "install": dict(
        port=22931,
        minutes=35,
        results=["install-result.json"],
        command=f"python3 {GUEST}/install_guest.py {COMMON} --output /root/qual/install-result.json",
    ),
    "container": dict(
        port=22932,
        minutes=25,
        results=["container-result.json", "container-result-fixture.json"],
        command=f"python3 {GUEST}/approved_guest.py --suite container {COMMON} --output /root/qual/container-result.json",
    ),
    "stack": dict(
        port=22933,
        minutes=60,
        results=["stack-result.json", "stack-result-fixture.json"],
        command=f"python3 {GUEST}/stack_guest.py --suite lifecycle {COMMON} --output /root/qual/stack-result.json",
    ),
    "scale": dict(
        port=22934,
        minutes=50,
        results=["scale-result.json", "scale-result-fixture.json"],
        command=f"python3 {GUEST}/stack_guest.py --suite scale {COMMON} --output /root/qual/scale-result.json",
    ),
}
FORBIDDEN = re.compile(
    r"docker|/opt/stacks|compose|pi-health|pihealth|smbd|systemctl\s+(start|stop|restart|enable|disable|mask|reload)"
)


class Refused(Exception):
    """Do not start (exit 3) or stop immediately (exit 4 once the host changed)."""


def utc_now():
    return datetime.now(UTC)


def parse_deadline(text):
    return native_vm.parse_utc(text)


# ---------------------------------------------------------------- pure checks
def parse_packages(snapshot):
    result = {}
    for line in snapshot["packages"]["stdout"].splitlines():
        name, _, version = line.partition("\t")
        if name:
            result[name] = version
    return result


def parse_simulation(text):
    """Return (installs, upgrades, removals) names from `apt-get -s` output."""
    installs, upgrades, removals = set(), set(), set()
    for line in text.splitlines():
        fields = line.split()
        if len(fields) < 2:
            continue
        if fields[0] == "Inst":
            (
                upgrades if len(fields) > 2 and fields[2].startswith("[") else installs
            ).add(fields[1].split(":")[0])
        elif fields[0] in ("Remv", "Purg"):
            removals.add(fields[1].split(":")[0])
    return installs, upgrades, removals


def listener_map(text):
    found = set()
    for line in text.splitlines()[1:]:
        fields = line.split()
        if len(fields) >= 5:
            found.add(
                (fields[3], tuple(sorted(set(re.findall(r'\(\("([^"]+)"', line)))))
            )
    return found


def compare_snapshots(pre, post):
    """Every difference that makes the host not identical to how we found it."""
    problems = []
    if parse_packages(pre) != parse_packages(post):
        problems.append("package list differs")
    if set(pre["manual_packages"]["stdout"].split()) != set(
        post["manual_packages"]["stdout"].split()
    ):
        problems.append("manually installed package set differs")
    units = lambda s: {
        l.split()[0] for l in s["running_units"]["stdout"].splitlines() if l.split()
    }
    if units(pre) != units(post):
        problems.append(f"running units differ: {sorted(units(pre) ^ units(post))}")
    before, after = (
        ph.parse_containers(pre["containers"]["stdout"]),
        ph.parse_containers(post["containers"]["stdout"]),
    )
    problems += ["docker: " + f for f in ph.container_failures(before, after)]
    names = lambda s: {
        l.split("\t")[1] for l in s["docker_ps"]["stdout"].splitlines() if "\t" in l
    }
    if names(pre) != names(post):
        problems.append(
            f"docker container names differ: {sorted(names(pre) ^ names(post))}"
        )
    if listener_map(pre["listeners"]["stdout"]) != listener_map(
        post["listeners"]["stdout"]
    ):
        problems.append("listening sockets differ (ss -tlnp)")
    if post["qemu_processes"]:
        problems.append("qemu or supervisor process remains")
    if pre["boot_id"] != post["boot_id"]:
        problems.append("host rebooted")
    if pre["vmstat"].get("oom_kill") != post["vmstat"].get("oom_kill"):
        problems.append("host OOM kill count changed")
    mem = post["meminfo"]
    if mem["SwapTotal"] - mem["SwapFree"] > ph.SWAP_MAX_KIB:
        problems.append("swap in use exceeds the guard limit afterwards")
    return problems


def watchdog_launch_command(deadline_epoch, entry="python3 -I pi_night_host.py watchdog", home=None):
    """Remote command that starts the watchdog detached and returns its pid at once.

    Only the daemon is backgrounded, inside a group: backgrounding the whole `&&` list
    leaves a subshell holding the SSH session's stdout, so ssh waits for the watchdog
    to exit and the call times out.
    """
    base = f"{home}/{NIGHT}" if home else f"~/{NIGHT}"
    return (
        f"cd {base} && rm -f abort.json done && {{ setsid nohup {entry} "
        f"--baseline baseline.json --log watchdog.jsonl --abort-flag abort.json --done-flag done "
        f"--deadline {deadline_epoch} </dev/null >watchdog.out 2>&1 & echo $!; }}"
    )


def start_refusals(snapshot):
    reasons = []
    if snapshot["uid"] == 0:
        reasons.append("run as the unprivileged SSH user, not root")
    if snapshot["containers"]["exit"]:
        reasons.append(
            "docker state unreadable: " + snapshot["containers"]["stderr"].strip()
        )
    rows = ph.parse_containers(snapshot["containers"]["stdout"])
    # Containers already stopped when the snapshot was taken (for example the
    # stopped Mattermost stack) are not part of the watchdog baseline; only
    # running ones are protected. Refuse only when nothing is running.
    if not any(r["status"] == "running" for r in rows.values()):
        reasons.append("no running containers visible; cannot baseline the watchdog")
    if snapshot["qemu_processes"]:
        reasons.append("a qemu or supervisor process already exists")
    reasons += ph.guard_failures(snapshot["meminfo"], snapshot["loadavg"], "start")
    return reasons


def check_deadline(deadline, authorization, now=None):
    now = now or utc_now()
    if deadline <= now + timedelta(minutes=LEAD_MINUTES + 5):
        raise Refused("deadline leaves no usable time")
    if authorization is None:
        raise Refused("--authorization is required for the Pi")
    if deadline + timedelta(minutes=TEARDOWN_MINUTES) > authorization["end"]:
        raise Refused(
            f"deadline must be {TEARDOWN_MINUTES} minutes before the authorization ends"
        )


def plan_suites(selected, deadline, now=None):
    """Suites that fit; the rest are skipped, never squeezed in."""
    now = now or utc_now()
    fits, clock = [], now + timedelta(minutes=8)  # boot margin for the first guest
    for name in selected:
        end = clock + timedelta(minutes=SUITES[name]["minutes"] + 4)
        if end + timedelta(minutes=LEAD_MINUTES) <= deadline:
            fits.append(name)
            clock = end
    return fits


# ------------------------------------------------------------------ transports
class Host:
    """Guarded SSH to the KVM host. Refuses anything near Docker or the frozen app."""

    def __init__(self, host, authorization, run_dir):
        self.guest = native_vm.Guest("pi-night", host, authorization)
        self.log = run_dir / "host-commands.jsonl"
        self.host = host

    def run(self, command, timeout=60, input=None):
        if FORBIDDEN.search(command):
            raise Refused(f"forbidden host command: {command[:80]}")
        started = utc_now().strftime("%Y-%m-%dT%H:%M:%SZ")
        done = self.guest.host_run(
            command,
            check=False,
            capture_output=True,
            text=True,
            timeout=timeout,
            input=input,
        )
        with self.log.open("a") as stream:
            stream.write(
                json.dumps(
                    {
                        "started": started,
                        "host": self.host,
                        "command": command,
                        "exit": done.returncode,
                        "stdout": done.stdout[-65536:],
                        "stderr": done.stderr[-8192:],
                    }
                )
                + "\n"
            )
        return done

    def helper(self, *arguments, timeout=90):
        """Run the read-only helper from its staged path."""
        return self.run(
            f"python3 -I ~/{NIGHT}/pi_night_host.py "
            + " ".join(map(shlex.quote, arguments)),
            timeout=timeout,
        )

    def push(self, source, destination):
        self.guest.host_push(source, destination)

    def pull(self, source, destination):
        self.guest.host_pull(source, destination)


class Vm:
    """native_vm.py as a subprocess: its supervisor and deadline logic stay authoritative."""

    def __init__(self, host, authorization, run_dir):
        self.base = [
            sys.executable,
            str(HERE / "native_vm.py"),
            "--host",
            host,
            "--authorization",
            str(authorization),
        ]
        self.log = run_dir / "vm-commands.jsonl"

    def call(self, *arguments, timeout=None, abort_check=None):
        started = utc_now().strftime("%Y-%m-%dT%H:%M:%SZ")
        process = subprocess.Popen(
            [*self.base, *arguments],
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
        )
        began = time.monotonic()
        aborted = False
        while True:
            try:
                out, _ = process.communicate(timeout=30)
                break
            except subprocess.TimeoutExpired:
                if (timeout and time.monotonic() - began > timeout) or (
                    abort_check and abort_check()
                ):
                    aborted = True
                    process.terminate()
                    try:
                        out, _ = process.communicate(timeout=15)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        out, _ = process.communicate()
                    break
        with self.log.open("a") as stream:
            stream.write(
                json.dumps(
                    {
                        "started": started,
                        "arguments": arguments,
                        "exit": process.returncode,
                        "aborted_by_orchestrator": aborted,
                        "output": out[-65536:],
                    }
                )
                + "\n"
            )
        return process.returncode, out, aborted


# ----------------------------------------------------------------- the night
class Night:
    def __init__(self, args, host, vm, deadline, authorization, sleep=time.sleep):
        self.args, self.host, self.vm = args, host, vm
        self.deadline, self.authorization, self.sleep = deadline, authorization, sleep
        self.run_dir = args.run_dir
        self.verdict = {
            "suites": {},
            "refusals": [],
            "problems": [],
            "added_packages": [],
        }
        self.pre = self.added = None
        self.staging_existed = True
        self.image_present = self.image_staged = False
        self.install_attempted = False
        self.watchdog_started = False
        self.abort_flag = f"~/{NIGHT}/abort.json"
        self.done_flag = f"~/{NIGHT}/done"
        self.watchdog_log = f"~/{NIGHT}/watchdog.jsonl"

    def save(self):
        (self.run_dir / "verdict.json").write_text(
            json.dumps(self.verdict, indent=2, sort_keys=True, default=str) + "\n"
        )

    def snapshot(self, name):
        done = self.host.run(
            "python3 -I - snapshot", input=Path(ph.__file__).read_text(), timeout=120
        )
        if done.returncode:
            raise Refused(f"snapshot failed: {done.stderr[-300:]}")
        data = json.loads(done.stdout)
        target = self.run_dir / f"host-{name}.json"
        if target.exists():
            target = target.with_name(f"host-{name}-{int(time.time())}.json")
        target.write_text(json.dumps(data, indent=2) + "\n")
        return data

    def aborted(self):
        return self.host.run(f"test -e {self.abort_flag}", timeout=40).returncode == 0

    def guard(self):
        done = self.host.helper("guard", "--phase", "start")
        record = json.loads(done.stdout or "{}")
        with (self.run_dir / "guards.jsonl").open("a") as stream:
            stream.write(json.dumps({"epoch": time.time(), **record}) + "\n")
        if done.returncode:
            raise Refused(
                "guard refused to start a guest: "
                + "; ".join(record.get("failures", ["guard failed"]))
            )

    # -- phases
    def prepare(self):
        pre = self.pre = self.snapshot("pre")
        reasons = start_refusals(pre)
        if reasons:
            self.verdict["refusals"] = reasons
            raise Refused("; ".join(reasons))
        self.staging_existed = pre["staging"]["exit"] == 0
        self.verdict["staging_existed"] = self.staging_existed
        self.host.run(f"mkdir -p ~/{NIGHT} ~/{STAGING}/image")
        self.host.push(Path(ph.__file__), f"{NIGHT}/pi_night_host.py")
        local_hash = hashlib.sha256(Path(ph.__file__).read_bytes()).hexdigest()
        remote = self.host.run(f"sha256sum ~/{NIGHT}/pi_night_host.py").stdout.split()[
            :1
        ]
        if remote != [local_hash]:
            raise Refused("staged host helper hash differs")
        self.guard()
        baseline = self.run_dir / "host-pre.json"
        self.host.push(baseline, f"{NIGHT}/baseline.json")
        deadline_epoch = self.deadline.timestamp()
        self.host.run(watchdog_launch_command(deadline_epoch))
        self.watchdog_started = True
        self.sleep(12)
        if (
            "watchdog_started" not in self.host.run(f"cat {self.watchdog_log}").stdout
            or self.aborted()
        ):
            raise Refused("watchdog did not start cleanly")
        self.install_qemu()
        self.stage_image()

    def install_qemu(self):
        names = " ".join(QEMU_PACKAGES)
        sim = self.host.run(
            f"sudo -n apt-get -s install --no-install-recommends {names}", timeout=180
        )
        (self.run_dir / "qemu-install-simulation.txt").write_text(
            sim.stdout + sim.stderr
        )
        if sim.returncode:
            raise Refused("QEMU install simulation failed")
        installs, upgrades, removals = parse_simulation(sim.stdout)
        if upgrades or removals:
            raise Refused(
                f"QEMU install would upgrade or remove existing packages: {sorted(upgrades | removals)}"
            )
        if installs:
            self.install_attempted = True
            done = self.host.run(
                f"sudo -n apt-get install -y --no-install-recommends {names}",
                timeout=1500,
            )
            (self.run_dir / "qemu-install.txt").write_text(done.stdout + done.stderr)
            if done.returncode:
                raise Refused("QEMU install failed; teardown will purge what was added")
        after = self.snapshot("after-qemu-install")
        before, now = parse_packages(self.pre), parse_packages(after)
        changed = [n for n in before if now.get(n) != before[n]]
        if changed:
            raise Refused(f"existing packages changed: {changed[:10]}")
        added = sorted(set(now) - set(before))
        self.verdict["qemu_install_simulated"] = sorted(installs)
        (self.run_dir / "host-packages-added.txt").write_text(
            "".join(f"{n} {now[n]}\n" for n in added)
        )
        if {n.split(":")[0] for n in added} != installs:
            self.verdict.setdefault("notes", []).append(
                "added packages differ from the simulation; teardown purges the observed set"
            )

    def stage_image(self):
        image = f"~/{STAGING}/image/debian-12-genericcloud-arm64.qcow2"
        have = self.host.run(f"sha512sum {image}", timeout=300)
        self.image_present = have.returncode == 0 and have.stdout.split()[:1] == [
            IMAGE_SHA512
        ]
        if not self.image_present:
            self.host.push(
                self.args.image, f"{STAGING}/image/debian-12-genericcloud-arm64.qcow2"
            )
            have = self.host.run(f"sha512sum {image}", timeout=300)
        self.image_staged = (
            have.stdout.split()[:1] == [IMAGE_SHA512] and not self.image_present
        )
        if have.stdout.split()[:1] != [IMAGE_SHA512]:
            raise Refused(
                "staged Debian image SHA-512 differs from the published value"
            )

    def prepare_guest(self, name):
        """Bundle, build record and repository reach the guest once each (the bundle is ~57 MB)."""
        cand = self.args.candidate
        call = self.vm.call
        steps = [
            ("mkdir", lambda: call("exec", name, "mkdir -p /root/qual")),
            (
                "inputs",
                lambda: call("push", name, str(cand / "inputs"), "/root/qual/inputs"),
            ),
            (
                "extract",
                lambda: call(
                    "exec", name, "cd /root/qual && tar -xzf inputs/source.tar.gz"
                ),
            ),
            # The external manifest (with bundle hash) replaces the internal copy.
            (
                "manifest",
                lambda: call(
                    "push",
                    name,
                    str(cand / "inputs/source-manifest.json"),
                    "/root/qual/source-manifest.json",
                ),
            ),
            (
                "build-result",
                lambda: call(
                    "push",
                    name,
                    str(cand / "build-result.json"),
                    "/root/qual/build-result.json",
                ),
            ),
            (
                "repo",
                lambda: call(
                    "exec", name, "cp -a /root/qual/inputs/repo /opt/limeos-repo"
                ),
            ),
            (
                "hashes",
                lambda: call(
                    "exec",
                    name,
                    "cp /root/qual/source/tests/fixtures/werkzeug-hashes.json /root/werkzeug-hashes.json",
                ),
            ),
        ]
        for label, step in steps:
            code, out, _ = step()
            if code:
                raise RuntimeError(
                    f"guest preparation step {label} failed ({code}): {out[-300:]}"
                )

    def run_suite(self, suite):
        spec = SUITES[suite]
        name = f"night-{suite}"
        record = self.verdict["suites"][suite] = {
            "started": utc_now().isoformat(),
            "summary": "incomplete",
        }
        self.save()
        try:
            self.guard()
        except Refused as refusal:
            record["summary"] = f"refused: {refusal}"
            raise
        remaining = (self.deadline - utc_now()).total_seconds()
        if remaining < (spec["minutes"] // 2 + LEAD_MINUTES) * 60:
            record["summary"] = "skipped: not enough time before the deadline"
            return
        out_dir = self.run_dir / name
        out_dir.mkdir(exist_ok=True)
        try:
            code, out, _ = self.vm.call(
                "boot",
                name,
                "--port",
                str(spec["port"]),
                "--cpus",
                str(CPUS),
                "--memory",
                str(MEMORY_MIB),
                "--disk",
                DISK,
                "--image-sha512",
                IMAGE_SHA512,
                "--timeout",
                "600",
                "--terminate-at",
                self.deadline.strftime("%Y-%m-%dT%H:%M:%SZ"),
                timeout=900,
            )
            if code:
                record.update(summary="boot failed", detail=out[-500:])
                return
            self.prepare_guest(name)
            budget = min(
                spec["minutes"] * 60, (self.deadline - utc_now()).total_seconds() - 120
            )
            code, out, killed = self.vm.call(
                "exec", name, spec["command"], timeout=budget, abort_check=self.aborted
            )
            (out_dir / "suite-output.txt").write_text(out)
            record["exit"] = code
            for result in spec["results"]:
                self.vm.call(
                    "pull", name, f"/root/qual/{result}", str(out_dir / result)
                )
            self.vm.call(
                "exec",
                name,
                'journalctl -u "limeos-*" --no-pager > /root/qual/limeos-journal.txt; '
                "journalctl -k --no-pager > /root/qual/kernel-journal.txt; systemctl --failed --no-pager > /root/qual/failed-units.txt; free -m > /root/qual/free.txt",
            )
            for extra in (
                "limeos-journal.txt",
                "kernel-journal.txt",
                "failed-units.txt",
                "free.txt",
            ):
                self.vm.call("pull", name, f"/root/qual/{extra}", str(out_dir / extra))
            first = out_dir / spec["results"][0]
            try:
                summary = json.loads(first.read_text()).get("summary")
            except (OSError, ValueError):
                summary = None
            record["summary"] = (
                "pass"
                if code == 0 and summary in ("pass", None) and first.exists()
                else "fail"
            )
            if killed:
                record["summary"] = (
                    "stopped by orchestrator (timeout or watchdog abort)"
                )
        except RuntimeError as error:
            record.update(summary="fail", detail=str(error))
        finally:
            record["finished"] = utc_now().isoformat()
            stop_code, stop_out, _ = self.vm.call("stop", name, timeout=300)
            if stop_code:
                record["stop_failed"] = stop_out[-500:]
                self.verdict["problems"].append(
                    f"guest {name} did not stop cleanly; stop it by hand on the host"
                )
            self.save()

    def teardown(self):
        problems = self.verdict["problems"]
        if self.watchdog_started:
            self.host.run(f"touch {self.done_flag}")
            self.sleep(self.args.interval + 2)
        if self.pre is not None:
            # The observed difference, not the plan, decides what is purged.
            mid = self.snapshot("pre-purge")
            before, now = parse_packages(self.pre), parse_packages(mid)
            changed = [n for n in before if now.get(n) != before[n]]
            if changed:
                problems.append(
                    f"pre-existing packages changed, nothing purged: {changed[:10]}"
                )
            added = sorted(set(now) - set(before))
            self.verdict["added_packages"] = added
            if added and not changed:
                names = " ".join(shlex.quote(n) for n in added)
                sim = self.host.run(f"sudo -n apt-get -s purge {names}", timeout=180)
                (self.run_dir / "host-purge-simulation.txt").write_text(sim.stdout)
                _, _, removals = parse_simulation(sim.stdout)
                if sim.returncode or not removals <= {n.split(":")[0] for n in added}:
                    problems.append(
                        f"purge simulation would remove more than was added: {sorted(removals)}; nothing purged"
                    )
                else:
                    done = self.host.run(
                        f"sudo -n apt-get purge -y {names}", timeout=900
                    )
                    (self.run_dir / "host-purge.txt").write_text(
                        done.stdout + done.stderr
                    )
                    if done.returncode:
                        problems.append("purge failed")
        if self.watchdog_started:
            for name in ("watchdog.jsonl", "watchdog.out", "abort.json"):
                if self.host.run(f"test -e ~/{NIGHT}/{name}").returncode == 0:
                    self.host.pull(f"{NIGHT}/{name}", self.run_dir / name)
            if (self.run_dir / "abort.json").exists():
                self.verdict["watchdog_abort"] = (
                    self.run_dir / "abort.json"
                ).read_text()
        # Only what this run created under the staging directory.
        remove = [f"~/{NIGHT}"]
        if not self.staging_existed:
            remove = [f"~/{STAGING}"]
        elif self.image_staged:
            remove.append(f"~/{STAGING}/image/debian-12-genericcloud-arm64.qcow2")
        self.host.run("rm -rf " + " ".join(remove))
        if self.pre is not None:
            problems += compare_snapshots(self.pre, self.snapshot("post"))
        self.save()

    def teardown_only(self):
        """Recover after a crashed workstation: guests, packages and staging, then the diff."""
        self.pre = json.loads((self.run_dir / "host-pre.json").read_text())
        previous = self.run_dir / "verdict.json"
        if previous.is_file():
            self.staging_existed = json.loads(previous.read_text()).get(
                "staging_existed", True
            )
        self.watchdog_started = True
        for guest in sorted(native_vm.STATE.glob("night-*")):
            if (guest / "guest.json").is_file():
                self.vm.call("stop", guest.name, timeout=300)
        self.teardown()
        self.verdict["exit"] = 4 if self.verdict["problems"] else 0
        self.save()
        return self.verdict["exit"]

    def execute(self):
        code = 0
        try:
            try:
                self.prepare()
                for suite in plan_suites(self.args.suites, self.deadline):
                    self.run_suite(suite)
                    if self.aborted():
                        self.verdict["problems"].append(
                            "watchdog aborted the run; no further guest started"
                        )
                        break
                for suite in self.args.suites:
                    self.verdict["suites"].setdefault(
                        suite,
                        {"summary": "skipped: not enough time before the deadline"},
                    )
                if any(
                    v["summary"] != "pass"
                    for v in self.verdict["suites"].values()
                    if not v["summary"].startswith("skipped")
                ):
                    code = 1
            except Refused as refusal:
                self.verdict["refused"] = str(refusal)
                code = 3
        finally:
            self.teardown()
        if self.verdict["problems"]:
            code = 4
        self.verdict["exit"] = code
        self.save()
        return code


# -------------------------------------------------------------------- driver
def check_inputs(args):
    """Local validation shared by --check and --execute. No SSH."""
    from qualification import load_build

    problems = []
    cand = args.candidate
    build = None
    try:
        build = load_build(cand / "build-result.json", cand / "inputs/repo")
    except Exception as error:  # noqa: BLE001 - report every reason
        problems.append(f"candidate build record invalid: {error}")
    for script in (
        "install_guest.py",
        "approved_guest.py",
        "stack_guest.py",
        "qualification.py",
        "native_vm.py",
        "guest_supervisor.py",
    ):
        if not (HERE / script).is_file():
            problems.append(f"missing {script}")
    if not args.image.is_file():
        problems.append(f"image missing: {args.image}")
    else:
        digest = hashlib.sha512()
        with args.image.open("rb") as stream:
            for block in iter(lambda: stream.read(1 << 20), b""):
                digest.update(block)
        if digest.hexdigest() != IMAGE_SHA512:
            problems.append("image SHA-512 differs from Debian's published value")
    unknown = set(args.suites) - set(SUITES)
    if unknown:
        problems.append(f"unknown suites: {sorted(unknown)}")
    return build, problems


def main(argv=None):
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    parser.add_argument("--host", required=True, help="SSH target, e.g. holly@wybie")
    parser.add_argument(
        "--authorization",
        type=Path,
        required=True,
        help="recorded window (native_vm format)",
    )
    parser.add_argument(
        "--deadline", required=True, help="hard UTC deadline, e.g. 2026-11-06T05:30:00Z"
    )
    parser.add_argument(
        "--candidate",
        type=Path,
        required=True,
        help="prepared directory with build-result.json and inputs/",
    )
    parser.add_argument(
        "--run-dir", type=Path, required=True, help="new local evidence directory"
    )
    parser.add_argument("--image", type=Path, default=DEFAULT_IMAGE)
    parser.add_argument("--suites", type=lambda s: s.split(","), default=list(SUITES))
    parser.add_argument("--interval", type=float, default=5.0)
    parser.add_argument(
        "--execute",
        action="store_true",
        help="contact the host; the default only validates",
    )
    parser.add_argument(
        "--teardown-only",
        action="store_true",
        help="with --execute and an existing --run-dir: stop leftover night guests, purge exactly "
        "the packages added since that run's host-pre.json and diff the host; for a crashed run",
    )
    args = parser.parse_args(argv)
    try:
        deadline = parse_deadline(args.deadline)
        authorization = native_vm.load_authorization(args.authorization, args.host)
        if not args.teardown_only:
            check_deadline(deadline, authorization)
    except (ValueError, Refused) as error:
        print(f"refused: {error}", file=sys.stderr)
        return 3 if isinstance(error, Refused) else 2
    build, problems = check_inputs(args)
    suites = plan_suites([s for s in args.suites if s in SUITES], deadline)
    plan = {
        "host": args.host,
        "deadline": args.deadline,
        "version": build and build["identity"]["package_version"],
        "source": build and build["identity"]["source_commit"],
        "suites": suites,
        "skipped_for_time": [s for s in args.suites if s not in suites],
        "guest": {
            "cpus": CPUS,
            "memory_mib": MEMORY_MIB,
            "disk": DISK,
            "nice": 19,
            "ionice": "idle",
        },
        "guard": {
            "start_min_available_mib": ph.START_MIN_AVAILABLE_KIB // 1024,
            "run_min_available_mib": ph.RUN_MIN_AVAILABLE_KIB // 1024,
            "swap_max_fraction": ph.SWAP_MAX_FRACTION,
            "swap_max_mib": ph.SWAP_MAX_KIB // 1024,
            "load_max": ph.LOAD_MAX,
        },
        "problems": problems,
    }
    print(json.dumps(plan, indent=2))
    if problems:
        return 2
    if not args.execute:
        print(
            "check only; nothing contacted. Add --execute for the night run.",
            file=sys.stderr,
        )
        return 0
    if args.teardown_only:
        if not (args.run_dir / "host-pre.json").is_file():
            print(
                "--teardown-only needs a run directory holding host-pre.json",
                file=sys.stderr,
            )
            return 2
        host = Host(args.host, args.authorization, args.run_dir)
        vm = Vm(args.host, args.authorization, args.run_dir)
        return Night(args, host, vm, deadline, authorization).teardown_only()
    if args.run_dir.exists():
        print("run directory exists; use a new one", file=sys.stderr)
        return 2
    args.run_dir.mkdir(parents=True)
    args.suites = suites
    args.candidate = args.candidate.resolve()
    host = Host(args.host, args.authorization, args.run_dir)
    vm = Vm(args.host, args.authorization, args.run_dir)
    return Night(args, host, vm, deadline, authorization).execute()


if __name__ == "__main__":
    sys.exit(main())
