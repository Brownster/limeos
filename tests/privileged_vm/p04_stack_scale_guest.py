#!/usr/bin/env python3
"""Whole-stack lifecycle on an Engine holding many unrelated containers; exclusively inside the disposable VM.

Reuses the lifecycle guest's installer, session, Compose-labelled fixtures, plan/approve/queue helpers and the
independent `docker events` proof. Cases: (1) 30 containers (15 running, many standalone, two multi-member
projects): the shared inventory is fresh and a stack restart plans, runs and verifies; (2) exactly 64 containers
(`MAX_ENGINE_CONTAINERS`) are fresh and still plan; (3) 65 containers fail closed (inventory not fresh, plans
refuse) and recover when one is removed. A sampler records the stack reader's cgroup memory, tasks and
descriptors against its limits (64 MiB, 16 tasks, 256 descriptors) for the whole run.
"""

import json
import os
import socket
import sys
import threading
import time
from pathlib import Path

import p03_guest as fx
import p04_stack_lifecycle_guest as sl

READER = "limeos-storage-reader"
CGROUP = Path("/sys/fs/cgroup/system.slice") / (READER + ".service")
LIMITS = {"memory_bytes": 64 * 1024 * 1024, "tasks": 16, "nofile": 256}
sl.order.clear()  # only this file's cases run; setup() still provisions `sl1` and the holder


class Sampler(threading.Thread):
    """Poll the reader cgroup every 20 ms. Reading /proc/<pid>/fd opens descriptors in this process only."""

    def __init__(self):
        super().__init__(daemon=True)
        self.stop = threading.Event()
        self.samples = 0
        self.peak = {"memory_current": 0, "tasks": 0, "fds_total": 0, "fds_one_process": 0, "processes": 0}

    def run(self):
        while not self.stop.is_set():
            try:
                pids = [int(p) for p in (CGROUP / "cgroup.procs").read_text().split()]
                fds = []
                for pid in pids:
                    try:
                        fds.append(len(os.listdir(f"/proc/{pid}/fd")))
                    except OSError:
                        pass
                current = int((CGROUP / "memory.current").read_text())
                tasks = int((CGROUP / "pids.current").read_text())
                self.samples += 1
                p = self.peak
                p["memory_current"] = max(p["memory_current"], current)
                p["tasks"] = max(p["tasks"], tasks)
                p["fds_total"] = max(p["fds_total"], sum(fds))
                p["fds_one_process"] = max(p["fds_one_process"], max(fds, default=0))
                p["processes"] = max(p["processes"], len(pids))
            except (OSError, ValueError):
                pass
            time.sleep(0.02)


sampler = Sampler()


def cgroup_facts():
    facts = {"limits": LIMITS}
    for name in ("memory.current", "memory.peak", "memory.max", "pids.current", "pids.peak", "pids.max", "memory.swap.current"):
        try:
            facts[name] = (CGROUP / name).read_text().strip()
        except OSError:
            facts[name] = None
    facts["sampler"] = {"samples": sampler.samples, **sampler.peak}
    return facts


def engine_total():
    return len(fx.docker("ps", "-aq").split())


def inventory():
    status, _, value = sl.http("/api/v1/deployment/stacks", cookie=sl.session["cookie"])
    return status, value


def make_standalone(prefix, count, running=0):
    ids = []
    for index in range(count):
        # `created` containers cost one inode and no process; the running ones are the small busybox loop.
        identifier = fx.docker("create", "--name", f"{prefix}-{index}", "--network=none", sl.IMAGE, *sl.LOOP)
        ids.append(identifier)
    for identifier in ids[:running]:
        fx.docker("start", identifier)
    return ids


def fresh_inventory(project, ids, timeout=120):
    sl.refresh_cohort(project, ids, timeout=timeout)
    status, value = inventory()
    sl.check(status == 200 and value["engine"]["state"] == "fresh", "inventory is not fresh", status=status)
    return value


def restart_and_verify(project, ids, key):
    before = {i: sl.started_at(i) for i in ids}
    proposal, approval = sl.approved_plan(project, "restart", ids)
    sl.check(
        [m["resource"] for m in proposal["plan"]["expected"]["members"]] == ["container:" + i for i in ids],
        "plan members are not exactly the stack's members",
        plan=proposal["plan"]["expected"],
    )
    mark = sl.mark()
    status, job, _ = sl.stack_queue(proposal, approval, key)
    sl.check(status == 202, "queue refused", status=status, job=job)
    sl.check(sl.wait_job(job["id"]) == "succeeded", "restart did not succeed", progress=sl.progress(job["id"]))
    value = sl.progress(job["id"])
    sl.check(value["receipt"]["state"] == "verified", "aggregate not verified", receipt=value["receipt"])
    sl.check(all(sl.fx.inspect(i)["State"]["StartedAt"] != before[i] for i in ids), "StartedAt unchanged")
    seen = sl.effects(mark, ids)
    sl.check(all(c.get("restart") == 1 for c in seen.values()), "not exactly one restart per member", events=seen)
    sl.check(sl.locks_consistent() == [], "locks left after success")
    return {"job": job["id"], "receipt": value["receipt"]["state"], "events": seen}


@sl.case("S1 stack restart among 30 containers (15 running, two multi-member projects, standalone)", retire=("sl2",))
def scale_30():
    ids = sl.members("sl1")
    other = sl.provision("sl2")
    for identifier in ids + other:
        if not sl.running(identifier):
            fx.docker("start", identifier)
    standalone = make_standalone("sc", 23, running=9)
    total = engine_total()
    running = len(fx.docker("ps", "-q").split())
    sl.check(total >= 24 and running > 8, "fixture too small", total=total, running=running)
    value = fresh_inventory("sl1", ids)
    sl.check(value["standalone_containers"] == total - 6, "standalone count", reported=value["standalone_containers"], total=total)
    detail = restart_and_verify("sl1", ids, "scale-30")
    detail.update(total=total, running=running, standalone_reported=value["standalone_containers"], other_projects=[p["project"]["name"] for p in value["other_projects"]], cgroup=cgroup_facts())
    fixture_ids = sl.fixture.setdefault("scale", [])
    fixture_ids.extend(standalone)
    return detail


@sl.case("S2 exactly 64 containers: fresh, plans and restarts")
def exactly_64():
    ids = sl.members("sl1")
    have = engine_total()
    sl.check(have <= 64, "already above the bound", have=have)
    sl.fixture.setdefault("scale", []).extend(make_standalone("fill", 64 - have))
    total = engine_total()
    sl.check(total == 64, "did not reach 64", total=total)
    value = fresh_inventory("sl1", ids)
    detail = restart_and_verify("sl1", ids, "scale-64")
    detail.update(total=total, running=len(fx.docker("ps", "-q").split()), standalone_reported=value["standalone_containers"], cgroup=cgroup_facts())
    return detail


@sl.case("S3 65 containers: inventory not fresh and plans refused (fail closed); recovers at 64")
def over_65():
    ids = sl.members("sl1")
    extra = make_standalone("over", 1)[0]
    sl.fixture.setdefault("scale", []).append(extra)
    sl.check(engine_total() == 65, "not 65")
    seen = {}

    def closed():
        status, value = inventory()
        seen["status"] = status
        if status == 200:
            seen["engine"] = value["engine"]["state"]
        plan_status, _ = sl.stack_plan("sl1", "restart")
        seen["plan"] = plan_status
        return seen.get("engine") != "fresh" and plan_status in (409, 503)

    fx.eventually(closed, timeout=240)
    final = dict(seen)
    for _ in range(3):  # stays closed: no flapping back to a partial view
        time.sleep(5)
        status, value = inventory()
        sl.check(status != 200 or value["engine"]["state"] != "fresh", "65 containers became fresh", value=value)
        plan_status, _ = sl.stack_plan("sl1", "restart")
        sl.check(plan_status in (409, 503), "plan accepted with 65 containers", status=plan_status)
    fx.docker("rm", "-f", extra)
    sl.fixture["scale"].remove(extra)
    sl.check(engine_total() == 64, "not back to 64")
    value = fresh_inventory("sl1", ids)
    return {"closed": final, "recovered_engine": value["engine"]["state"], "cgroup": cgroup_facts()}


def main():
    if os.getuid() != 0 or socket.gethostname() != "limeos-p01-test":
        raise SystemExit("Disposable guest required")
    original = sl.memory_facts

    def with_reader():
        facts = original()
        facts["reader_cgroup"] = cgroup_facts()
        return facts

    sl.memory_facts = with_reader
    # Start sampling once the reader exists; the sampler tolerates a missing cgroup.
    sampler.start()
    try:
        sl.main(package_version=os.environ.get("LIMEOS_PACKAGE_VERSION", "0.4.15+stacklc.1"))
    finally:
        sampler.stop.set()
        print("SCALE_READER " + json.dumps(cgroup_facts()), flush=True)


if __name__ == "__main__":
    main()
