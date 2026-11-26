#!/usr/bin/env python3
"""Whole-stack lifecycle on an Engine holding many unrelated containers; exclusively inside the disposable VM.

Reuses the lifecycle guest's installer, session, Compose-labelled fixtures, plan/approve/queue helpers and the
independent `docker events` proof. Cases: (1) 30 containers (15 running, many standalone, two multi-member
projects): the shared inventory is fresh and a stack restart plans, runs and verifies; (2) exactly 64 containers
(`MAX_ENGINE_CONTAINERS`) are fresh and still plan; (3) 65 containers degrade, not refuse (PR-15): the inventory
stays fresh with `truncated_containers == 1`, keeping running containers first, so the running stack is complete
and still plans; removing one clears the flag. A sampler records the stack reader's cgroup memory, tasks and
descriptors against its limits (64 MiB, 16 tasks, 256 descriptors) for the whole run.

Each case also has core make a container-health read through the reader (`limeosctl reader-status --read
container-health`, as the cutover's activation does) on an Engine holding a health mix: running containers
with a passing and a failing check, and `restart: always` containers stopped after their check failed, plus
a stopped unchecked one. At 30 and 64 containers the read is accepted whole; at 65 it is accepted with a
`truncated: 1` reason, never `unavailable`; back at 64 the reason clears. An 18-container Engine (14
running) once refused this read at its 16-container bound and rolled a migration back.
"""

import json
import os
import socket
import sys
import tarfile
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


LIMEOSCTL = "/usr/lib/limeos/limeosctl"
# The fixture image with `/bin/sh`, which Docker's `--health-cmd` (CMD-SHELL) needs.
HEALTH_IMAGE = "limeos-sl-health:local"


def health_image():
    if sl.fixture.get("health_image"):
        return
    rootfs = Path("/root/sl-health-rootfs.tar")
    with tarfile.open(rootfs, "w") as archive:
        archive.add("/bin/busybox", arcname="bin/busybox")
        shell = tarfile.TarInfo("bin/sh")
        shell.type = tarfile.SYMTYPE
        shell.linkname = "busybox"
        archive.addfile(shell)
    fx.docker("import", str(rootfs), HEALTH_IMAGE)
    sl.fixture["health_image"] = True


def health_status(identifier):
    return (fx.inspect(identifier)["State"].get("Health") or {}).get("Status")


def make_health_mix(prefix):
    """Six standalone containers, three running: two with a passing check, one with a failing check; and three
    stopped: two `restart: always` whose check had failed (alerted as not running) and one without a check."""
    health_image()

    def create(name, *flags):
        return fx.docker("create", "--name", f"{prefix}-{name}", "--network=none", *flags, HEALTH_IMAGE, *sl.LOOP)

    passing = ("--health-cmd", "exit 0", "--health-interval", "1s", "--health-retries", "1")
    failing = ("--health-cmd", "exit 1", "--health-interval", "1s", "--health-retries", "1")
    mix = {
        "healthy": [create("ok-0", *passing), create("ok-1", *passing)],
        "unhealthy": [create("bad-0", *failing)],
        "stopped_unhealthy": [create("gone-0", "--restart", "always", *failing), create("gone-1", "--restart", "always", *failing)],
        "stopped": [create("off-0")],
    }
    for identifier in mix["healthy"] + mix["unhealthy"] + mix["stopped_unhealthy"]:
        fx.docker("start", identifier)
    fx.eventually(
        lambda: all(health_status(i) == "healthy" for i in mix["healthy"])
        and all(health_status(i) == "unhealthy" for i in mix["unhealthy"] + mix["stopped_unhealthy"]),
        timeout=60,
    )
    for identifier in mix["stopped_unhealthy"]:
        fx.docker("stop", "--time", "2", identifier)
    stopped = mix["stopped_unhealthy"] + mix["stopped"]
    sl.check(all(not fx.inspect(i)["State"]["Running"] for i in stopped), "health mix did not stop")
    mix["statuses"] = {i: health_status(i) for group in ("healthy", "unhealthy", "stopped_unhealthy") for i in mix[group]}
    return mix


def mix_ids(mix):
    return [i for group in ("healthy", "unhealthy", "stopped_unhealthy", "stopped") for i in mix[group]]


def reader_journal():
    return fx.run("journalctl", "-u", READER, "--no-pager", "-n", "30", check=False).stdout[-4000:]


def container_health_read(truncated=0):
    """Core's container-health read through the reader, as `scripts/cutover/activate.sh` makes it: accepted, and
    with `truncated` > 0 accepted with that many containers reported left out, never failed or unavailable."""
    attempts = []
    for _ in range(3):
        result = fx.run(LIMEOSCTL, "reader-status", "--read", "container-health", check=False, timeout=60)
        try:
            answer = json.loads(result.stdout)
        except ValueError:
            answer = {"stdout": result.stdout[-2000:], "stderr": result.stderr[-2000:]}
        attempts.append({"rc": result.returncode, "answer": answer})
        if result.returncode == 0 and (answer.get("read") or {}).get("result") == "accepted":
            break
        time.sleep(2)
    last = attempts[-1]
    read = last["answer"].get("read") or {}
    sl.check(
        last["rc"] == 0 and read.get("result") == "accepted",
        "container-health read through the reader was not accepted",
        attempts=attempts,
        engine=engine_total(),
        journal=reader_journal(),
    )
    reason = read.get("reason") or ""
    if truncated:
        sl.check(f"truncated: {truncated} container(s)" in reason, "truncation not reported", read=read)
    else:
        sl.check(read.get("reason") is None, "a whole read gave a reason", read=read)
    return {"attempts": len(attempts), "read": read, "engine": engine_total(), "cgroup": cgroup_facts()}


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


@sl.case("S1 stack restart among 30 containers (15 running, two multi-member projects, standalone, a health mix); container-health read accepted", retire=("sl2",))
def scale_30():
    ids = sl.members("sl1")
    other = sl.provision("sl2")
    for identifier in ids + other:
        if not sl.running(identifier):
            fx.docker("start", identifier)
    # 23 standalone, 9 running: 17 plain (6 running) and the six-container health mix (3 running).
    standalone = make_standalone("sc", 17, running=6)
    mix = make_health_mix("hm")
    standalone.extend(mix_ids(mix))
    total = engine_total()
    running = len(fx.docker("ps", "-q").split())
    sl.check(total >= 24 and running > 8, "fixture too small", total=total, running=running)
    value = fresh_inventory("sl1", ids)
    sl.check(value["standalone_containers"] == total - 6, "standalone count", reported=value["standalone_containers"], total=total)
    fixture_ids = sl.fixture.setdefault("scale", [])
    fixture_ids.extend(standalone)
    detail = restart_and_verify("sl1", ids, "scale-30")
    detail.update(total=total, running=running, standalone_reported=value["standalone_containers"], other_projects=[p["project"]["name"] for p in value["other_projects"]], health_mix=mix["statuses"])
    detail["container_health"] = container_health_read()
    detail["cgroup"] = cgroup_facts()
    return detail


@sl.case("S2 exactly 64 containers: fresh, plans and restarts; container-health read accepted whole")
def exactly_64():
    ids = sl.members("sl1")
    have = engine_total()
    sl.check(have <= 64, "already above the bound", have=have)
    sl.fixture.setdefault("scale", []).extend(make_standalone("fill", 64 - have))
    total = engine_total()
    sl.check(total == 64, "did not reach 64", total=total)
    value = fresh_inventory("sl1", ids)
    detail = restart_and_verify("sl1", ids, "scale-64")
    detail.update(total=total, running=len(fx.docker("ps", "-q").split()), standalone_reported=value["standalone_containers"])
    detail["container_health"] = container_health_read()
    detail["cgroup"] = cgroup_facts()
    return detail


@sl.case("S3 65 containers: inventory stays fresh with a truncation flag, the running stack still plans and the container-health read is accepted truncated")
def over_65():
    ids = sl.members("sl1")
    extra = make_standalone("over", 1)[0]
    sl.fixture.setdefault("scale", []).append(extra)
    sl.check(engine_total() == 65, "not 65")
    seen = {}

    def flagged():
        status, value = inventory()
        seen["status"] = status
        if status == 200:
            seen["engine"] = value["engine"]["state"]
            seen["truncated"] = value.get("truncated_containers")
        return seen.get("engine") == "fresh" and seen.get("truncated") == 1

    fx.eventually(flagged, timeout=240)
    final = dict(seen)
    for _ in range(3):  # stays degraded, not refused: no flapping to unavailable
        time.sleep(5)
        status, value = inventory()
        sl.check(status == 200 and value["engine"]["state"] == "fresh", "65 containers blanked the inventory", status=status)
        sl.check(value.get("truncated_containers") == 1, "truncation flag missing", value=value)
    # The stopped standalone container is the one left out; the running stack is whole.
    plan_status, proposal = sl.stack_plan("sl1", "restart", retry=True)
    sl.check(plan_status == 201, "running stack did not plan with 65 containers", status=plan_status)
    sl.check(
        [m["resource"] for m in proposal["plan"]["expected"]["members"]] == ["container:" + i for i in ids],
        "plan members are not exactly the stack's members",
        plan=proposal["plan"]["expected"],
    )
    # Past the bound the read is answered, not refused: one container (a stopped one) left out and said so.
    truncated_health = container_health_read(truncated=1)
    fx.docker("rm", "-f", extra)
    sl.fixture["scale"].remove(extra)
    sl.check(engine_total() == 64, "not back to 64")
    value = fresh_inventory("sl1", ids)
    sl.check(value.get("truncated_containers") == 0, "truncation flag not cleared at 64", value=value)
    whole_health = container_health_read()
    return {
        "degraded": final,
        "plan": plan_status,
        "recovered_engine": value["engine"]["state"],
        "container_health": {"at_65": truncated_health, "back_at_64": whole_health},
        "cgroup": cgroup_facts(),
    }


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
