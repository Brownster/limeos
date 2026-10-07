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
import os
import select
import signal
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


def main():
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
    try:
        # A pidfd names this exact process; it can never signal a reused PID.
        pidfd = os.pidfd_open(args.pid)
    except OSError:
        pidfd = None
    if pidfd is None or not matches(args.pid, args.start_ticks, args.uid, args.marker):
        log("refused_unbound", **bound)
        return 2
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
