#!/usr/bin/env python3
"""Harness self-check workload: exercises descriptor, task and memory ceilings.

This is not a qualification probe and touches no Docker, process or storage
evidence. It proves that the reproduced confinement actually enforces the
storage reader's ceilings in the guest, and that the external sampler sees
descriptors and memory. Prints progress lines, then one JSON summary line.
"""

import argparse
import json
import os
import threading
import time


def emit(record):
    print(json.dumps(record, sort_keys=True), flush=True)


def descriptors(target):
    baseline = len(os.listdir("/proc/self/fd")) - 1  # the listing's own descriptor
    held, error = [], None
    try:
        while len(held) < target:
            held.append(os.open("/dev/null", os.O_RDONLY))
    except OSError as failure:
        error = failure.strerror
    count = baseline + len(held)  # counting needs no further descriptor at the limit
    time.sleep(0.5)  # let the external sampler observe the peak
    for fd in held:
        os.close(fd)
    return {
        "requested": target,
        "opened": len(held),
        "fd_table_at_peak": count,
        "error": error,
    }


def tasks(target):
    stop = threading.Event()
    started, error = [], None
    try:
        while len(started) < target:
            thread = threading.Thread(target=stop.wait, daemon=True)
            thread.start()
            started.append(thread)
    except RuntimeError as failure:
        error = str(failure)
    time.sleep(0.5)
    stop.set()
    return {"requested": target, "started": len(started), "error": error}


def memory(target_mib):
    chunks = []
    for index in range(target_mib // 4):
        chunk = bytearray(4 << 20)
        for offset in range(0, len(chunk), 4096):
            chunk[offset] = 1
        chunks.append(chunk)
        emit({"memory_mib_touched": (index + 1) * 4})
    time.sleep(0.5)
    return {"requested_mib": target_mib, "touched_mib": len(chunks) * 4}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--fds", type=int, default=0)
    parser.add_argument("--tasks", type=int, default=0)
    parser.add_argument("--memory-mib", type=int, default=0)
    args = parser.parse_args()
    summary = {"pid": os.getpid()}
    if args.fds:
        summary["fds"] = descriptors(args.fds)
    if args.tasks:
        summary["tasks"] = tasks(args.tasks)
    if args.memory_mib:
        summary["memory"] = memory(args.memory_mib)
    with open("/proc/self/status") as stream:
        summary["vm_hwm"] = next(
            line.split()[1] for line in stream if line.startswith("VmHWM:")
        )
    emit({"summary": summary})


if __name__ == "__main__":
    main()
