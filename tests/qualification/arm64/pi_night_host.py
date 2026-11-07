#!/usr/bin/env python3
"""Read-only host snapshot, start/mid-run guard and docker watchdog for the Pi night run.

Runs ON the KVM host (Debian 12 / Raspberry Pi OS, python3 stdlib only), as the
SSH user. It never writes outside the paths it is given, never uses sudo, never
starts, stops or restarts any container, and never touches /opt/stacks or the
frozen application. The only process it signals is a guest QEMU of this
harness (`-name limeos-arm64-*`, same UID, qemu-system-aarch64), via a pidfd
after re-checking the kernel start time.

    pi_night_host.py snapshot             -> JSON on stdout
    pi_night_host.py guard --phase start  -> JSON; exit 3 when a guest must not start
    pi_night_host.py watchdog ...         -> long-running; see --help

Thresholds below are the contract; the workstation can only tighten them.
"""

import argparse
import json
import os
import signal
import subprocess
import sys
import time
from pathlib import Path

KIB = 1024
GIB = 1024 * 1024  # in KiB
# Starting a guest: the previous run's two-GiB floor is raised to three.
START_MIN_AVAILABLE_KIB = 3 * GIB
# Mid-run: the previous run's floor, since the guest itself holds 1.5 GiB.
RUN_MIN_AVAILABLE_KIB = 2 * GIB
SWAP_MAX_FRACTION = 0.90
SWAP_MAX_KIB = 450 * 1024  # the previous run's absolute swap guard
LOAD_MAX = 2.0
TEMPERATURE_MAX_MILLIDEGREES = 75000
GUEST_PREFIX = "limeos-arm64-"
INSPECT_FORMAT = (
    "{{.Id}}|{{.Name}}|{{.State.Status}}|{{.State.StartedAt}}|{{.RestartCount}}"
)


# Test-only: a directory laid out like / that stands in for the kernel statistics
# files below (never set on the real host; process inspection stays on the real /proc).
FAKE_ROOT_ENV = "LIMEOS_QUAL_PROC_ROOT"
FAKE_ROOT_PREFIXES = ("/proc/meminfo", "/proc/loadavg", "/proc/vmstat", "/proc/sys/", "/sys/")


def read(path):
    fake = os.environ.get(FAKE_ROOT_ENV)
    if fake and str(path).startswith(FAKE_ROOT_PREFIXES):
        path = fake + str(path)
    try:
        return Path(path).read_text()
    except OSError:
        return ""


def parse_meminfo(text):
    return {
        line.split(":")[0]: int(line.split()[1])
        for line in text.splitlines()
        if ":" in line and len(line.split()) >= 2 and line.split()[1].isdigit()
    }


def guard_failures(mem, loadavg, phase="start"):
    """Reasons a guest must not start (phase start) or must be stopped (phase run)."""
    failures = []
    available = mem.get("MemAvailable", 0)
    floor = START_MIN_AVAILABLE_KIB if phase == "start" else RUN_MIN_AVAILABLE_KIB
    if available < floor:
        failures.append(
            f"MemAvailable {available // 1024} MiB below {floor // 1024} MiB"
        )
    total, free = mem.get("SwapTotal", 0), mem.get("SwapFree", 0)
    used = total - free
    if total and used > SWAP_MAX_FRACTION * total:
        failures.append(
            f"swap used {used // 1024} MiB is over 90% of {total // 1024} MiB"
        )
    if used > SWAP_MAX_KIB:
        failures.append(
            f"swap used {used // 1024} MiB exceeds {SWAP_MAX_KIB // 1024} MiB"
        )
    if phase == "start":
        try:
            loads = [float(x) for x in loadavg.split()[:2]]
        except ValueError:
            loads = [float("inf")]
        if not loads or max(loads) > LOAD_MAX:
            failures.append(f"load average {loads} exceeds {LOAD_MAX}")
    return failures


def parse_containers(text):
    rows = {}
    for line in text.splitlines():
        parts = line.strip().split("|")
        if len(parts) == 5:
            rows[parts[0]] = {
                "name": parts[1].lstrip("/"),
                "status": parts[2],
                "started_at": parts[3],
                "restarts": parts[4],
            }
    return rows


def container_failures(baseline, current):
    """Containers of the baseline that stopped, vanished, were recreated or restarted."""
    failures = []
    for ident, before in baseline.items():
        if before["status"] != "running":
            continue  # already stopped when we found it: the run refuses to start on those
        now = current.get(ident)
        if now is None:
            failures.append(
                f"{before['name']}: container {ident[:12]} no longer exists"
            )
        elif now["status"] != "running":
            failures.append(f"{before['name']}: state {now['status']}")
        elif (
            now["started_at"] != before["started_at"]
            or now["restarts"] != before["restarts"]
        ):
            failures.append(
                f"{before['name']}: restarted (StartedAt/RestartCount changed)"
            )
    return failures


def command(*argv, timeout=20):
    try:
        done = subprocess.run(
            argv, capture_output=True, text=True, timeout=timeout, check=False
        )
        return {
            "exit": done.returncode,
            "stdout": done.stdout,
            "stderr": done.stderr[-4000:],
        }
    except (OSError, subprocess.SubprocessError) as error:
        return {"exit": 127, "stdout": "", "stderr": repr(error)}


def containers():
    listed = command("docker", "ps", "-aq", "--no-trunc")
    if listed["exit"]:
        return {"exit": listed["exit"], "stdout": "", "stderr": listed["stderr"]}
    ids = listed["stdout"].split()
    if not ids:
        return {"exit": 0, "stdout": "", "stderr": ""}
    return command("docker", "inspect", "--format", INSPECT_FORMAT, *ids, timeout=30)


def qemu_processes():
    found = []
    for directory in Path("/proc").iterdir():
        if not directory.name.isdigit():
            continue
        try:
            argv = [
                a.decode(errors="replace")
                for a in (directory / "cmdline").read_bytes().split(b"\0")
                if a
            ]
            stat = (directory / "stat").read_text()
            fields = stat[stat.rindex(")") + 2 :].split()
            uid = int(
                next(
                    l
                    for l in (directory / "status").read_text().splitlines()
                    if l.startswith("Uid:")
                ).split()[1]
            )
        except (OSError, ValueError, IndexError, StopIteration):
            continue
        if fields[0] == "Z" or not argv:
            continue
        if Path(argv[0]).name == "qemu-system-aarch64" or any(
            Path(a).name == "guest_supervisor.py" for a in argv[:4]
        ):
            found.append(
                {
                    "pid": int(directory.name),
                    "start_ticks": int(fields[19]),
                    "uid": uid,
                    "argv": argv,
                }
            )
    return found


def snapshot():
    mem = parse_meminfo(read("/proc/meminfo"))
    vmstat = {
        l.split()[0]: int(l.split()[1])
        for l in read("/proc/vmstat").splitlines()
        if l.startswith(("oom_kill ", "pswpin ", "pswpout "))
    }
    temperature = read("/sys/class/thermal/thermal_zone0/temp").strip()
    return {
        "captured_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "captured_epoch": time.time(),
        "boot_id": read("/proc/sys/kernel/random/boot_id").strip(),
        "uid": os.getuid(),
        "uname": command("uname", "-a")["stdout"].strip(),
        "meminfo": mem,
        "loadavg": read("/proc/loadavg").strip(),
        "vmstat": vmstat,
        "temperature_millidegrees": int(temperature) if temperature.isdigit() else None,
        "free_m": command("free", "-m")["stdout"],
        "packages": command(
            "dpkg-query",
            "-W",
            "-f",
            "${Package}:${Architecture}\t${Version}\n",
            timeout=60,
        ),
        "manual_packages": command("apt-mark", "showmanual", timeout=60),
        "running_units": command(
            "systemctl",
            "list-units",
            "--type=service",
            "--state=running",
            "--plain",
            "--no-legend",
            "--no-pager",
        ),
        "containers": containers(),
        "docker_ps": command(
            "docker",
            "ps",
            "-a",
            "--no-trunc",
            "--format",
            "{{.ID}}\t{{.Names}}\t{{.State}}\t{{.RunningFor}}\t{{.Status}}",
        ),
        "listeners": command("ss", "-tlnp"),
        "qemu_processes": qemu_processes(),
        "staging": command("ls", "-la", str(Path.home() / "limeos-arm64-qual")),
    }


def event(log, name, **extra):
    record = {
        "utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "epoch": round(time.time(), 3),
        "event": name,
        **extra,
    }
    with Path(log).open("a") as stream:
        stream.write(json.dumps(record) + "\n")


def stop_guests(log, grace=20):
    """SIGTERM then SIGKILL the harness's own guest QEMU processes (never anything else)."""
    me = os.getuid()
    for process in qemu_processes():
        argv = process["argv"]
        if (
            process["uid"] != me
            or Path(argv[0]).name != "qemu-system-aarch64"
            or "-name" not in argv
            or not argv[argv.index("-name") + 1 :][:1]
            or not argv[argv.index("-name") + 1].startswith(GUEST_PREFIX)
        ):
            continue
        try:
            pidfd = os.pidfd_open(process["pid"])
        except OSError:
            continue
        try:
            stat = Path(f"/proc/{process['pid']}/stat").read_text()
            if int(stat[stat.rindex(")") + 2 :].split()[19]) != process["start_ticks"]:
                continue
            signal.pidfd_send_signal(pidfd, signal.SIGTERM)
            event(
                log,
                "guest_sigterm",
                pid=process["pid"],
                name=argv[argv.index("-name") + 1],
            )
            deadline = time.time() + grace
            while time.time() < deadline and Path(f"/proc/{process['pid']}").exists():
                time.sleep(0.5)
            if Path(f"/proc/{process['pid']}").exists():
                signal.pidfd_send_signal(pidfd, signal.SIGKILL)
                event(log, "guest_sigkill", pid=process["pid"])
        except (OSError, ValueError, ProcessLookupError):
            pass
        finally:
            os.close(pidfd)


def attribution(log, since):
    """Evidence for working out who stopped a container; read-only, bounded."""
    evidence = {
        "docker_events": command(
            "docker",
            "events",
            "--since",
            str(int(since)),
            "--until",
            str(int(time.time()) + 1),
            "--filter",
            "type=container",
            "--format",
            "{{.Time}} {{.Action}} {{.Actor.Attributes.name}} {{json .Actor.Attributes}}",
            timeout=30,
        ),
        "docker_ps": command(
            "docker",
            "ps",
            "-a",
            "--no-trunc",
            "--format",
            "{{.ID}}\t{{.Names}}\t{{.State}}\t{{.Status}}",
        ),
        "docker_socket_users": command("ss", "-xpn"),
        "processes": command(
            "ps", "-eo", "pid,ppid,user,lstart,args", "--sort=start_time"
        ),
        "docker_journal": command(
            "journalctl",
            "-u",
            "docker",
            "--since",
            f"@{int(since)}",
            "--no-pager",
            "-n",
            "200",
        ),
        "dmesg_oom": command(
            "sh", "-c", "dmesg 2>/dev/null | grep -i -E 'oom|killed process' | tail -20"
        ),
    }
    event(
        log,
        "attribution",
        **{
            k: {
                "exit": v["exit"],
                "stdout": v["stdout"][-12000:],
                "stderr": v["stderr"][-1000:],
            }
            for k, v in evidence.items()
        },
    )


def watchdog(args):
    log = args.log
    baseline_snapshot = json.loads(Path(args.baseline).read_text())
    baseline = parse_containers(baseline_snapshot["containers"]["stdout"])
    if not baseline:
        event(log, "refused", reason="baseline has no containers")
        return 3
    started = time.time()
    event(
        log,
        "watchdog_started",
        pid=os.getpid(),
        deadline=args.deadline,
        containers=sorted(v["name"] for v in baseline.values()),
        boot_id=baseline_snapshot["boot_id"],
        baseline_oom=baseline_snapshot["vmstat"].get("oom_kill"),
    )
    last_beat = 0.0
    while True:
        now = time.time()
        if Path(args.done_flag).exists():
            event(log, "done_flag")
            return 0
        if now >= args.deadline:
            event(log, "deadline_reached")
            return 0
        failures = []
        current = containers()
        if current["exit"]:
            failures.append("docker ps failed: " + current["stderr"][-200:].strip())
        else:
            failures += container_failures(
                baseline, parse_containers(current["stdout"])
            )
        mem = parse_meminfo(read("/proc/meminfo"))
        failures += guard_failures(mem, read("/proc/loadavg"), phase="run")
        vm = {
            l.split()[0]: int(l.split()[1])
            for l in read("/proc/vmstat").splitlines()
            if l.startswith("oom_kill ")
        }
        if vm.get("oom_kill") != baseline_snapshot["vmstat"].get("oom_kill"):
            failures.append("new host OOM kill")
        temperature = read("/sys/class/thermal/thermal_zone0/temp").strip()
        if temperature.isdigit() and int(temperature) >= TEMPERATURE_MAX_MILLIDEGREES:
            failures.append(f"temperature {int(temperature) / 1000} C")
        if (
            read("/proc/sys/kernel/random/boot_id").strip()
            != baseline_snapshot["boot_id"]
        ):
            failures.append("host rebooted")
        if failures:
            event(
                log,
                "abort",
                failures=failures,
                seconds_since_start=round(now - started, 1),
            )
            Path(args.abort_flag).write_text(
                json.dumps({"epoch": now, "failures": failures}) + "\n"
            )
            attribution(log, baseline_snapshot["captured_epoch"])
            stop_guests(log)
            return 4
        if now - last_beat >= args.heartbeat:
            last_beat = now
            event(
                log,
                "ok",
                available_mib=mem.get("MemAvailable", 0) // 1024,
                swap_used_mib=(mem.get("SwapTotal", 0) - mem.get("SwapFree", 0))
                // 1024,
            )
        time.sleep(args.interval)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    sub.add_parser("snapshot")
    g = sub.add_parser("guard")
    g.add_argument("--phase", choices=["start", "run"], default="start")
    w = sub.add_parser("watchdog")
    w.add_argument(
        "--baseline", required=True, help="snapshot JSON taken before any change"
    )
    w.add_argument("--log", required=True)
    w.add_argument("--abort-flag", required=True)
    w.add_argument("--done-flag", required=True)
    w.add_argument("--deadline", type=float, required=True, help="epoch seconds")
    w.add_argument("--interval", type=float, default=5.0)
    w.add_argument("--heartbeat", type=float, default=60.0)
    args = parser.parse_args()
    if args.command == "snapshot":
        print(json.dumps(snapshot(), indent=1))
        return 0
    if args.command == "guard":
        failures = guard_failures(
            parse_meminfo(read("/proc/meminfo")), read("/proc/loadavg"), args.phase
        )
        print(
            json.dumps(
                {
                    "phase": args.phase,
                    "refused": bool(failures),
                    "failures": failures,
                    "meminfo_available_kib": parse_meminfo(read("/proc/meminfo")).get(
                        "MemAvailable"
                    ),
                    "loadavg": read("/proc/loadavg").strip(),
                }
            )
        )
        return 3 if failures else 0
    return watchdog(args)


if __name__ == "__main__":
    sys.exit(main())
