#!/usr/bin/env python3
"""Stop one exact QEMU guest at a deadline; never signal any other process.

Started detached on the KVM host next to the guest it supervises, so losing the
workstation or SSH cannot leave the guest running past an authorized window.
The guest is identified by PID, kernel start time, real UID and an exact
argument (the unique `-name` marker the runner passes to QEMU). If any of these
differ, the PID was reused or the guest already stopped: exit, sending nothing.

At --terminate-at the guest receives SIGTERM; if it is still the same process at
--kill-at it receives SIGKILL. Every decision is logged as one JSON line.
Uses only the Python standard library (Debian 12's python3).
"""

import argparse
import json
import math
import os
import select
import signal
import stat
import subprocess
import sys
import time
from pathlib import Path


def identity(pid):
    """Return (start ticks, real uid, argv) for a live PID, or None."""
    try:
        stat = Path(f"/proc/{pid}/stat").read_text()
        # comm may contain spaces or parentheses; fields resume after the last ')'.
        fields = stat[stat.rindex(")") + 2 :].split()
        start = int(fields[19])
        status = Path(f"/proc/{pid}/status").read_text()
        uid = int(
            next(l for l in status.splitlines() if l.startswith("Uid:")).split()[1]
        )
        argv = Path(f"/proc/{pid}/cmdline").read_bytes().split(b"\0")
        if fields[0] == "Z":
            return None
        return start, uid, [a.decode(errors="replace") for a in argv if a]
    except (OSError, ValueError, IndexError, StopIteration):
        return None


def matches(pid, start, uid, marker):
    found = identity(pid)
    return (
        found is not None
        and found[0] == start
        and found[1] == uid
        and marker in found[2]
    )


def bind_process(record):
    """Hold the exact process before inspecting its numeric PID."""
    try:
        pidfd = os.pidfd_open(record["pid"])
    except OSError:
        return None
    if not matches(
        record["pid"], record["start_ticks"], record["uid"], record["marker"]
    ):
        os.close(pidfd)
        return None
    return pidfd


def wait_process(pidfd, seconds):
    exited = select.poll()
    exited.register(pidfd, select.POLLIN)
    return bool(exited.poll(max(0, int(seconds * 1000))))


def log_event(path, event, **extra):
    stamp = time.time()
    record = {
        "utc": time.strftime("%Y-%m-%dT%H:%M:%S", time.gmtime(stamp))
        + f".{int(stamp % 1 * 1000):03d}Z",
        "epoch": round(stamp, 3),
        "event": event,
        **extra,
    }
    with Path(path).open("a") as stream:
        stream.write(json.dumps(record) + "\n")


def monitor(pidfd, args):
    terminated = False
    while True:
        now = time.time()
        try:
            if now >= args.kill_at:
                signal.pidfd_send_signal(pidfd, signal.SIGKILL)
                log_event(args.log, "sigkill")
                gone = wait_process(pidfd, 10)
                log_event(
                    args.log, "guest_gone" if gone else "kill_failed", signalled=True
                )
                return 0 if gone else 1
            if now >= args.terminate_at and not terminated:
                signal.pidfd_send_signal(pidfd, signal.SIGTERM)
                terminated = True
                log_event(args.log, "sigterm")
        except ProcessLookupError:
            log_event(args.log, "guest_gone", signalled=terminated)
            return 0
        next_event = args.kill_at if terminated else args.terminate_at
        if wait_process(pidfd, min(args.poll, max(0.05, next_event - time.time()))):
            log_event(args.log, "guest_gone", signalled=terminated)
            return 0


def launch_owned(argv, cwd, args):
    """Own a foreground child from birth; all failure paths kill and reap it."""
    if not (
        all(math.isfinite(t) for t in (args.terminate_at, args.kill_at, args.not_after))
        and time.time() < args.terminate_at < args.kill_at < args.not_after
        and 0 < args.poll <= 10
    ):
        raise ValueError("usable deadlines inside the authorization required")
    # Prove logging is usable before any guest can exist. Later failures still
    # enter the owned-child finally block; logging is never needed for cleanup.
    log_event(args.log, "launching", marker=args.marker)
    owner = identity(os.getpid())
    if owner is None:
        raise RuntimeError("launcher identity unavailable")
    (args.state.parent / "owner.json").write_text(
        json.dumps(
            {
                "pid": os.getpid(),
                "start_ticks": owner[0],
                "uid": owner[1],
                "marker": args.marker,
            }
        )
        + "\n"
    )
    # Logging/state I/O may cross a boundary. Refuse immediately before fork,
    # independently of any earlier workstation-side authorization decision.
    if time.time() >= args.terminate_at:
        raise ValueError("launch deadline passed before process creation")
    child = subprocess.Popen(
        argv,
        cwd=cwd,
        start_new_session=True,
        env={"PATH": "/usr/sbin:/usr/bin:/sbin:/bin", "LC_ALL": "C"},
        stdin=subprocess.DEVNULL,
    )
    pidfd = None
    try:
        # There is no daemonizing parent to lose. Until wait()/poll() reaps it,
        # this owned child PID cannot be reused even if pidfd_open fails.
        pidfd = os.pidfd_open(child.pid)
        ready_until = min(
            time.monotonic() + 10, time.monotonic() + args.terminate_at - time.time()
        )
        while time.monotonic() < ready_until:
            found = identity(child.pid)
            if found and found[1] == args.uid and args.marker in found[2]:
                record = {
                    "pid": child.pid,
                    "start_ticks": found[0],
                    "uid": args.uid,
                    "marker": args.marker,
                }
                # Publish only after the retained process identity matches.
                with args.state.open("x") as stream:
                    stream.write(json.dumps(record) + "\n")
                log_event(
                    args.log,
                    "bound",
                    supervisor_pid=os.getpid(),
                    terminate_at=args.terminate_at,
                    kill_at=args.kill_at,
                    **record,
                )
                return monitor(pidfd, args)
            if wait_process(pidfd, 0.05):
                raise RuntimeError("owned guest exited before identity publication")
        raise RuntimeError(
            "owned guest failed to establish its identity before deadline"
        )
    finally:
        if child.poll() is None:
            try:
                if pidfd is None:
                    child.kill()
                else:
                    signal.pidfd_send_signal(pidfd, signal.SIGKILL)
            except ProcessLookupError:
                pass
        try:
            child.wait(timeout=10)
        finally:
            if pidfd is not None:
                os.close(pidfd)


def controlled_json(path):
    """Read a file from the launcher's private directory without following links."""
    directory = path.parent
    owner = os.geteuid()
    parent = directory.lstat()
    if (
        not stat.S_ISDIR(parent.st_mode)
        or parent.st_uid != owner
        or parent.st_mode & 0o077
    ):
        raise ValueError("private launcher directory required")
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC)
    try:
        info = os.fstat(fd)
        if (
            not stat.S_ISREG(info.st_mode)
            or info.st_uid != owner
            or info.st_nlink != 1
            or info.st_mode & 0o022
            or info.st_size > 16384
        ):
            raise ValueError("protected bounded launch configuration required")
        with os.fdopen(fd, "r", closefd=False) as stream:
            return json.load(stream)
    finally:
        os.close(fd)


def launch_main(argv):
    parser = argparse.ArgumentParser(
        description="Launch and supervise one foreground guest"
    )
    parser.add_argument("--command", type=Path, required=True)
    parser.add_argument("--state", type=Path, required=True)
    parser.add_argument("--log", type=Path, required=True)
    parser.add_argument("--uid", type=int, required=True)
    parser.add_argument("--marker", required=True)
    parser.add_argument("--terminate-at", type=float, required=True)
    parser.add_argument("--kill-at", type=float, required=True)
    parser.add_argument("--not-after", type=float, required=True)
    parser.add_argument("--poll", type=float, default=2)
    args = parser.parse_args(argv)
    spec = controlled_json(args.command)
    prefix = [
        "/usr/bin/nice",
        "-n",
        "19",
        "/usr/bin/ionice",
        "-c",
        "3",
        "/usr/bin/qemu-system-aarch64",
    ]
    if (
        not isinstance(spec, dict)
        or set(spec) != {"argv", "cwd"}
        or not isinstance(spec["argv"], list)
        or spec["argv"][:7] != prefix
        or not all(isinstance(a, str) and "\0" not in a for a in spec["argv"])
        or any(a in {"-daemonize", "--daemonize"} for a in spec["argv"])
        or not isinstance(spec["cwd"], str)
        or not Path(spec["cwd"]).is_absolute()
        or args.uid < 0
        or ["-name", args.marker]
        not in [spec["argv"][i : i + 2] for i in range(len(spec["argv"]) - 1)]
        or args.state.parent != args.command.parent
        or args.log.parent != args.command.parent
    ):
        raise ValueError(
            "closed foreground QEMU command and private state/log required"
        )
    os.umask(0o077)
    signal.signal(signal.SIGHUP, signal.SIG_IGN)
    return launch_owned(spec["argv"], spec["cwd"], args)


def process_record(path, expected_marker=None):
    """Stop only a recorded process; a reused PID or substring cannot match."""
    record = controlled_json(Path(path))
    if (
        not isinstance(record, dict)
        or set(record) != {"pid", "start_ticks", "uid", "marker"}
        or any(type(record[k]) is not int for k in ("pid", "start_ticks", "uid"))
        or record["pid"] <= 1
        or record["start_ticks"] <= 0
        or record["uid"] < 0
        or not isinstance(record["marker"], str)
        or not record["marker"]
    ):
        raise ValueError("complete process identity required")
    if expected_marker is not None and record["marker"] != expected_marker:
        raise ValueError("recorded guest marker differs")
    return record


def stop_record(path, grace=30, expected_marker=None):
    """Stop only a recorded process; a reused PID or substring cannot match."""
    record = process_record(path, expected_marker)
    pidfd = bind_process(record)
    if pidfd is None:
        return 0 if identity(record["pid"]) is None else 2
    try:
        signal.pidfd_send_signal(pidfd, signal.SIGTERM)
        if wait_process(pidfd, grace):
            return 0
        signal.pidfd_send_signal(pidfd, signal.SIGKILL)
        return 0 if wait_process(pidfd, 10) else 1
    except ProcessLookupError:
        return 0
    finally:
        os.close(pidfd)


def wait_record(path, marker):
    record = process_record(path, marker)
    pidfd = bind_process(record)
    if pidfd is None:
        return 0 if identity(record["pid"]) is None else 2
    try:
        return 0 if wait_process(pidfd, 30) else 1
    finally:
        os.close(pidfd)


def main():
    if len(sys.argv) > 1 and sys.argv[1] == "launch":
        return launch_main(sys.argv[2:])
    if len(sys.argv) > 1 and sys.argv[1] in {"stop", "wait"}:
        parser = argparse.ArgumentParser(
            description="Stop or wait for one recorded process"
        )
        parser.add_argument("state", type=Path)
        parser.add_argument("--marker", required=True)
        args = parser.parse_args(sys.argv[2:])
        if sys.argv[1] == "stop":
            return stop_record(args.state, expected_marker=args.marker)
        return wait_record(args.state, args.marker)
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pid", type=int, required=True)
    parser.add_argument("--start-ticks", type=int, required=True)
    parser.add_argument("--uid", type=int, required=True)
    parser.add_argument(
        "--marker", required=True, help="Exact argv element unique to this guest"
    )
    parser.add_argument(
        "--terminate-at", type=float, required=True, help="UTC epoch seconds"
    )
    parser.add_argument(
        "--kill-at", type=float, required=True, help="UTC epoch seconds"
    )
    parser.add_argument("--log", type=Path, required=True)
    parser.add_argument(
        "--state", type=Path, help="Publish the verified process identity"
    )
    parser.add_argument("--poll", type=float, default=2.0)
    args = parser.parse_args()
    if not (args.pid > 1 and args.kill_at > args.terminate_at and 0 < args.poll <= 10):
        parser.error("pid > 1, kill-at after terminate-at and 0 < poll <= 10 required")

    def log(event, **extra):
        # time.gmtime() without an argument reads the coarse clock, which can still
        # show the previous second just after a deadline; stamp from time.time().
        stamp = time.time()
        record = {
            "utc": time.strftime("%Y-%m-%dT%H:%M:%S", time.gmtime(stamp))
            + f".{int(stamp % 1 * 1000):03d}Z",
            "epoch": round(stamp, 3),
            "event": event,
            **extra,
        }
        with args.log.open("a") as stream:
            stream.write(json.dumps(record) + "\n")

    bound = {
        "pid": args.pid,
        "start_ticks": args.start_ticks,
        "uid": args.uid,
        "marker": args.marker,
    }
    pidfd = bind_process(bound)
    if pidfd is None:
        log("refused_unbound", **bound)
        return 2
    if args.state:
        args.state.write_text(json.dumps(bound) + "\n")
    log(
        "bound",
        supervisor_pid=os.getpid(),
        terminate_at=args.terminate_at,
        kill_at=args.kill_at,
        **bound,
    )
    exited = select.poll()
    exited.register(pidfd, select.POLLIN)
    terminated = False

    def wait(seconds):
        """True once the bound process has exited."""
        return bool(exited.poll(max(0, int(seconds * 1000))))

    while True:
        now = time.time()
        try:
            if now >= args.kill_at:
                signal.pidfd_send_signal(pidfd, signal.SIGKILL)
                log("sigkill")
                gone = wait(10)
                log("guest_gone" if gone else "kill_failed", signalled=True)
                return 0 if gone else 1
            if now >= args.terminate_at and not terminated:
                signal.pidfd_send_signal(pidfd, signal.SIGTERM)
                terminated = True
                log("sigterm")
        except ProcessLookupError:
            log("guest_gone", signalled=terminated)
            return 0
        next_event = args.kill_at if terminated else args.terminate_at
        if wait(min(args.poll, max(0.05, next_event - time.time()))):
            log("guest_gone", signalled=terminated)
            return 0


if __name__ == "__main__":
    sys.exit(main())
