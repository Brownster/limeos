# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Bounded post-refusal diagnostics; never admission or failed-scan facts."""
import json
import os
from pathlib import Path
import time

MAX_TASKS = 256
MAX_NAMESPACES = 32
MAX_OUTPUT = 24 * 1024

def snapshot():
    started = time.monotonic()
    groups = {}
    errors = []
    error_count = 0
    seen = 0
    kernel_tasks = 0
    user_tasks = []
    complete = True

    def poll():
        if time.monotonic() - started >= 2:
            raise ValueError("diagnostic deadline")

    def ids(path):
        values = []
        with os.scandir(path) as entries:
            for count, entry in enumerate(entries):
                poll()
                if count >= 512:
                    raise ValueError("diagnostic directory bound")
                if entry.name.isascii() and entry.name.isdecimal():
                    if len(values) >= MAX_TASKS:
                        raise ValueError("diagnostic task bound")
                    values.append(int(entry.name))
        return sorted(values)

    def read(path, maximum):
        with path.open("rb") as stream:
            value = stream.read(maximum + 1)
        if len(value) > maximum:
            raise ValueError("diagnostic field bound")
        return value.decode("utf-8")

    def key(path):
        status = path.stat()
        return status.st_dev, status.st_ino

    def error(pid, tid, failure):
        nonlocal complete, error_count
        complete = False
        error_count += 1
        if len(errors) < 8:
            errors.append({"pid": pid, "tid": tid, "type": type(failure).__name__, "errno": getattr(failure, "errno", None)})

    try:
        host = key(Path("/proc/1/ns/mnt"))
        pids = ids(Path("/proc"))
        for pid in pids:
            poll()
            proc = Path("/proc") / str(pid)
            try:
                tids = ids(proc / "task")
                cgroup = read(proc / "cgroup", 1024)
                paths = [line[3:] for line in cgroup.splitlines() if line.startswith("0::")]
                if len(paths) != 1 or len(paths[0]) > 256:
                    raise ValueError("diagnostic cgroup shape")
                for tid in tids:
                    poll()
                    if seen >= MAX_TASKS:
                        raise ValueError("diagnostic total task bound")
                    seen += 1
                    task = proc / "task" / str(tid)
                    try:
                        text = read(task / "stat", 4096)
                        fields = text[text.rindex(")") + 1:].split()
                        flags = int(fields[6])
                        if flags & 0x0020_0000:
                            kernel_tasks += 1
                            continue
                        identity = key(task / "ns/mnt")
                        user_tasks.append({"pid": pid, "tid": tid, "start": int(fields[19]), "namespace": list(identity), "cgroup": paths[0]})
                        if identity not in groups:
                            if len(groups) >= MAX_NAMESPACES:
                                raise ValueError("diagnostic namespace bound")
                            groups[identity] = {"device": identity[0], "inode": identity[1], "host": identity == host, "task_count": 0, "task_samples": [], "cgroup_samples": []}
                        group = groups[identity]
                        group["task_count"] += 1
                        if len(group["task_samples"]) < 4:
                            group["task_samples"].append({"pid": pid, "tid": tid})
                        if paths[0] not in group["cgroup_samples"] and len(group["cgroup_samples"]) < 2:
                            group["cgroup_samples"].append(paths[0])
                    except (OSError, ValueError, UnicodeError, IndexError) as failure:
                        error(pid, tid, failure)
                if tids != ids(proc / "task"):
                    error(pid, None, ValueError("task membership changed"))
            except (OSError, ValueError, UnicodeError) as failure:
                error(pid, None, failure)
        if pids != ids(Path("/proc")):
            error(None, None, ValueError("process membership changed"))
    except (OSError, ValueError) as failure:
        error(None, None, failure)
    result = {
        "scope": "later post-refusal diagnostic only; no authority or failed-scan sample",
        "complete_without_observed_read_or_membership_errors": complete,
        "tasks_seen": seen,
        "kernel_tasks_seen": kernel_tasks,
        "namespace_count_seen": len(groups),
        "namespaces": list(groups.values()),
        "user_tasks": user_tasks,
        "error_count": error_count,
        "error_samples": errors,
        "elapsed_seconds": time.monotonic() - started,
        "bounds": {"tasks": MAX_TASKS, "namespaces": MAX_NAMESPACES, "bytes": MAX_OUTPUT, "seconds": 2},
    }
    if len(json.dumps(result).encode()) > MAX_OUTPUT:
        return {"scope": result["scope"], "complete_without_observed_read_or_membership_errors": False, "error": "diagnostic output bound"}
    return result
