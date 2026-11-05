#!/usr/bin/env python3
"""Whole-stack start/stop/restart qualification; exclusively inside the disposable Debian VM.

Drives the real HTTP API (`/api/v1/stack/*`) with an enrolled session and CSRF token against a
genuine Docker Engine, Compose-labelled ordinary busybox services, the installed packages and the
real container executor. `docker events` supplies independent proof of every Engine effect. The Engine is the
packaged dockerd on its own listener (no proxy): the stack reader authenticates the Engine peer. Members that
ignore SIGTERM make a restart take the Engine's full 10 s grace, giving deterministic mid-call kill windows.
Each case records pass/fail and continues; the full result is printed between STACK_RESULT markers.
"""

import hashlib
import json
import os
import pwd
import signal
import socket
import sqlite3
import subprocess
import sys
import tarfile
import time
import traceback
from collections import Counter
from pathlib import Path

import p03_guest as fx
import p04_mounts_guest as mounts

STACK = "/api/v1/stack"
CONTAINER = "/api/v1/container"
ROOT = Path("/opt/limeos-sl-fixture")
CATALOG = Path("/etc/limeos/management-catalog.json")
POLICY = Path("/etc/limeos/system-policy/container.json")
IMAGE = "limeos-sl:local"
SERVICES = ("web", "worker", "cache")
PROJECTS = ("sl1", "sl2", "sl3", "sl4", "sl5", "sl6", "sl7", "sl8")
SLOW_PROJECTS = ("sl4", "sl5", "sl6")
LOOP = ["/bin/busybox", "sh", "-c", "trap 'exit 0' TERM INT; while :; do /bin/busybox sleep 1; done"]
SLOW = ["/bin/busybox", "sleep", "3600"]  # PID 1 ignores SIGTERM: stop takes the full 10 s grace
TERMINAL = {"succeeded", "failed", "canceled", "precondition_changed", "needs_intervention"}
HELD = ("running", "verifying", "outcome_unknown", "needs_intervention")
EVENTS = Path("/root/docker-events.jsonl")

session = {}
fixture = {"stacks": {}, "extra": []}
results = {}
order = []


# ---------------------------------------------------------------- helpers
def check(condition, message, **facts):
    if not condition:
        raise AssertionError(message + (" " + json.dumps(facts, default=str)[:1500] if facts else ""))


def case(name, retire=()):
    def decorate(function):
        def run():
            started = time.monotonic()
            try:
                detail = function() or {}
                results[name] = {"status": "pass", "detail": detail}
                fx.passed(name)
            except BaseException as error:  # noqa: BLE001 - recorded, never hidden
                results[name] = {
                    "status": "fail",
                    "error": repr(error)[:2500],
                    "trace": traceback.format_exc()[-3000:],
                }
                print("FAIL " + name + ": " + repr(error)[:1500], flush=True)
            finally:
                for project in retire:
                    try:
                        retire_stack(project)
                    except Exception as error:  # noqa: BLE001
                        results[name]["retire_error"] = repr(error)[:300]
                results[name]["seconds"] = round(time.monotonic() - started, 1)
                try:
                    fx.wait_container_ready(timeout=45)
                    wait_core()
                except Exception as error:  # noqa: BLE001
                    results[name]["service_recovery_error"] = repr(error)[:500]

        order.append(run)
        return function

    return decorate


def http(path, method="GET", body=None, cookie=None, csrf=None, **kw):
    cookie = cookie if cookie is not None else session["cookie"]
    csrf = csrf if csrf is not None else (session["csrf"] if method != "GET" else None)
    return fx.http(path, method, body, cookie, csrf, **kw)


def wait_core():
    fx.eventually(lambda: fx.http("/api/v1/overview", cookie=session["cookie"])[0] == 200, timeout=60)


def unit_pid(unit):
    return int(fx.run("systemctl", "show", unit, "-p", "MainPID", "--value").stdout)


def kill_unit(unit):
    pid = unit_pid(unit)
    os.kill(pid, signal.SIGKILL)
    return pid


def running(identifier):
    return fx.inspect(identifier)["State"]["Running"]


def started_at(identifier):
    return fx.inspect(identifier)["State"]["StartedAt"]


def job_state(job):
    return fx.state(job)


def wait_job(job, wanted=TERMINAL, timeout=90):
    last = {}

    def done():
        last["state"] = job_state(job)
        return last["state"] in wanted

    try:
        fx.eventually(done, timeout=timeout)
    except AssertionError:
        raise AssertionError(f"job {job} stuck in {last.get('state')} (wanted {sorted(wanted)})") from None
    return last["state"]


def progress(job):
    status, _, value = http(f"{STACK}/jobs/{job}")
    check(status == 200, "progress read failed", status=status, value=value)
    return value


def mark():
    time.sleep(1.2)  # let `docker events` deliver everything already emitted
    return len(EVENTS.read_text().splitlines()) if EVENTS.exists() else 0


def effects(since, ids):
    """Per-container counters of Engine lifecycle events after `since`."""
    time.sleep(1.2)
    result = {identifier: Counter() for identifier in ids}
    for line in EVENTS.read_text().splitlines()[since:]:
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            continue
        identifier = event.get("id") or event.get("Actor", {}).get("ID")
        action = (event.get("Action") or event.get("status") or "").split(":")[0]
        if identifier in result:
            result[identifier][action] += 1
    return {identifier: dict(counter) for identifier, counter in result.items()}


def wait_event(identifier, action, since, timeout=60):
    """Block until `docker events` shows `action` for the container after offset `since`."""
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        for line in EVENTS.read_text().splitlines()[since:]:
            try:
                event = json.loads(line)
            except json.JSONDecodeError:
                continue
            if event.get("id") == identifier and (event.get("Action") or event.get("status") or "").split(":")[0] == action:
                return True
        time.sleep(0.05)
    raise AssertionError(f"no {action} event for {identifier[:12]} within {timeout}s")


def engine_attempt_state(identifier):
    """Engine-side facts that change on every start attempt: error text, exit code and finish time."""
    state = fx.inspect(identifier)["State"]
    return {"status": state["Status"], "error": state.get("Error", ""), "exit_code": state["ExitCode"], "finished_at": state["FinishedAt"], "started_at": state["StartedAt"]}


def locks():
    with sqlite3.connect(fx.CORE_DB) as db:
        return db.execute(
            "SELECT l.resource,l.job,j.state FROM resource_locks l JOIN jobs j ON j.id=l.job"
        ).fetchall()


def locks_consistent():
    rows = locks()
    bad = [row for row in rows if row[2] not in HELD]
    check(not bad, "terminal job still owns a lock", rows=bad)
    return rows


def receipts(job):
    with sqlite3.connect(fx.RECEIPTS) as db:
        stack = db.execute("SELECT state,operation FROM stack_actions WHERE action=?", (job,)).fetchall()
        members = db.execute("SELECT resource,state,operation FROM actions WHERE action=?", (job,)).fetchall()
    return {"stack": stack, "member_rows": sorted(members)}


def refresh_cohort(project, ids, timeout=90):
    """Wait until the shared Stack inventory agrees with independent Engine truth."""
    last = {}

    def agreed():
        status, _, value = http("/api/v1/deployment/stacks", cookie=session["cookie"])
        last.clear()
        last["status"] = status
        if status != 200:
            last["body"] = value
            return False
        last.update(catalog=value["catalog"], engine=value["engine"], matches=value.get("engine_matches_catalog"))
        if value["catalog"]["state"] != "fresh" or value["engine"]["state"] != "fresh":
            return False
        row = next((s for s in value["stacks"] if s["project"] == project), None)
        if row is None:
            last["row"] = "missing"
            return False
        last.update(issues=row["issues"], runtime=row["runtime"], counts=row["counts"])
        if row["issues"]:
            return False
        seen = {
            m["container"]["resource"]: m["container"]["running"]
            for service in row["services"]
            for m in service["members"]
            if not m["one_off"]
        }
        want = {"container:" + i: running(i) for i in ids}
        last.update(seen=seen, want=want)
        return seen == want

    try:
        fx.eventually(agreed, timeout=timeout)
    except AssertionError:
        raise AssertionError("stack inventory never agreed with the Engine: " + json.dumps(last, default=str)[:1800]) from None


def stack_plan(project, operation, retry=False):
    deadline = time.monotonic() + (90 if retry else 0)
    while True:
        status, _, proposal = http(f"{STACK}/plans", "POST", {"project": project, "operation": operation})
        if status == 201 or status not in (409, 503) or time.monotonic() >= deadline:
            return status, proposal
        time.sleep(2)


def stack_approve(proposal):
    status, _, approval = http(
        f"{STACK}/plans/{proposal['plan']['id']}/approval", "POST", {"digest": proposal["digest"]}
    )
    check(status == 200, "approval refused", status=status, approval=approval)
    return approval["token"]


def stack_queue(proposal, approval, key):
    body = {"proposal": proposal, "approval": approval, "idempotency_key": key}
    status, _, job = http(f"{STACK}/jobs", "POST", body)
    return status, job, body


def approved_plan(project, operation, ids):
    refresh_cohort(project, ids)
    status, proposal = stack_plan(project, operation, retry=True)
    check(status == 201, f"{operation} plan on {project} refused", status=status, body=proposal)
    return proposal, stack_approve(proposal)


def container_plan(identifier, operation="restart"):
    status, _, proposal = http(
        f"{CONTAINER}/plans", "POST", {"resource": "container:" + identifier, "operation": operation}
    )
    check(status == 201, "container plan refused", status=status, body=proposal)
    status, _, approval = http(
        f"{CONTAINER}/plans/{proposal['plan']['id']}/approval", "POST", {"digest": proposal["digest"]}
    )
    check(status == 200, "container approval refused", status=status)
    return proposal, approval["token"]


def container_queue(proposal, approval, key):
    body = {"proposal": proposal, "approval": approval, "idempotency_key": key}
    status, _, job = http(f"{CONTAINER}/jobs", "POST", body)
    return status, job


def members(project):
    return fixture["stacks"][project]


def isolate(project, want_running):
    """Keep the Engine inside the stack reader's bound of 8 running containers.

    Only `project` (and, for the dispatcher-hold case, the holder) may run. These hand-made Engine changes
    precede every `mark()`, so they never count as LimeOS effects. `want_running=None` leaves `project` as is.
    """
    for other, ids in fixture["stacks"].items():
        if other != project:
            for identifier in ids:
                if running(identifier):
                    fx.docker("stop", "-t", "1", identifier)
    if project != "holder" and running(fixture["holder"]):
        fx.docker("stop", "-t", "1", fixture["holder"])
    if want_running is not None:
        for identifier in members(project):
            if running(identifier) != want_running:
                fx.docker("start" if want_running else "stop", *([] if want_running else ["-t", "1"]), identifier)


# ---------------------------------------------------------------- fixture
def create_member(project, service, index, command=None, name=None):
    directory = ROOT / project
    labels = [
        "com.docker.compose.project=" + project,
        "com.docker.compose.service=" + service,
        "com.docker.compose.project.working_dir=" + str(directory),
        "com.docker.compose.project.config_files=" + str(directory / "compose.yaml"),
        "com.docker.compose.oneoff=False",
    ]
    arguments = [a for label in labels for a in ("--label", label)]
    return fx.docker(
        "create",
        "--name",
        name or f"{project}-{service}-{index}",
        "--network=none",
        *arguments,
        IMAGE,
        *(command or LOOP),
    )


def make_stack(project, run=True, bad_middle=False, slow=False):
    ids = []
    if bad_middle:
        bad = create_member(project, "worker", 0, ["/bin/limeos-no-such-binary"])
        low = high = None
        extras = []
        for n in range(60):
            candidate = create_member(project, "web" if low is None else "cache", n)
            if candidate < bad and low is None:
                low = candidate
            elif candidate > bad and high is None:
                high = candidate
            else:
                extras.append(candidate)
            if low and high:
                break
        for extra in extras:
            fx.docker("rm", "-f", extra)
        check(low and high, "could not place the failing member in the middle")
        ids = sorted([low, bad, high])
    else:
        ids = sorted(create_member(project, service, 0, SLOW if slow else None) for service in SERVICES)
    if run:
        for identifier in ids:
            fx.docker("start", identifier)
    fixture["stacks"][project] = ids
    return ids


def provision(project, **kwargs):
    """Create one stack's members on demand: the stack reader accepts at most 16 Engine containers in total."""
    if project in fixture["stacks"]:
        return fixture["stacks"][project]
    ids = make_stack(project, run=False, **kwargs)
    fixture["managed"] = fixture.get("managed", []) + ids
    write_policy(managed_containers=sorted(set(fixture["managed"])))
    return ids


def retire_stack(project):
    for identifier in fixture["stacks"].pop(project, []):
        fx.docker("rm", "-f", identifier)


def catalog_value(engine_id):
    stacks = []
    for project in PROJECTS:
        directory = ROOT / project
        stacks.append(
            {
                # One registered stack name deliberately differs from its Compose project.
                "stack": "cat-" + project if project == "sl1" else project,
                "project": project,
                "working_directory": str(directory),
                "operator": {
                    "name": "compose.yaml",
                    "sha256": hashlib.sha256((directory / "compose.yaml").read_bytes()).hexdigest(),
                },
                "managed": {"state": "absent"},
                "services": list(SERVICES),
            }
        )
    return {"version": 1, "engine_id": engine_id, "stacks": stacks}


def write_policy(**changes):
    policy = json.loads(POLICY.read_text())
    policy.update(changes)
    POLICY.write_text(json.dumps(policy))
    fx.restart_container()
    return policy


def setup(repository, version):
    fx.run("apt-get", "update", "-qq")
    fx.run("apt-get", "install", "--no-install-recommends", "-y", "busybox-static")
    fx.run("install", "-m", "0644", str(repository / "limeos-archive-keyring.gpg"), "/usr/share/keyrings/limeos-test.gpg")
    Path("/etc/apt/sources.list.d/limeos-test.list").write_text(
        f"deb [signed-by=/usr/share/keyrings/limeos-test.gpg] file:{repository} stable main\n"
    )
    fx.run("apt-get", "update", "-qq")
    fx.run("apt-get", "install", "-y", "limeos=" + version)
    cookie, csrf, principal = fx.enroll()
    session.update(cookie=cookie, csrf=csrf, principal=principal)
    # The packaged dockerd on its own listener, exactly as the A1 mount fixture acquires it: the stack
    # reader authenticates the Engine peer, so a pass-through proxy cannot be used here.
    fixture["engine"] = mounts.actual_docker()
    fx.REAL_SOCKET = "/run/docker.sock"
    rootfs = Path("/root/sl-rootfs.tar")
    with tarfile.open(rootfs, "w") as archive:
        archive.add("/bin/busybox", arcname="bin/busybox")
    fx.docker("import", str(rootfs), IMAGE)
    EVENTS.write_text("")
    subprocess.Popen(
        ["docker", "--host", "unix://" + fx.REAL_SOCKET, "events", "--format", "{{json .}}", "--filter", "type=container"],
        stdout=EVENTS.open("ab"),
        stderr=subprocess.DEVNULL,
    )
    ROOT.mkdir(mode=0o755)
    for project in PROJECTS:
        directory = ROOT / project
        directory.mkdir(mode=0o755)
        (directory / "compose.yaml").write_text("services:\n  web:\n    image: fixture\n")
        (directory / "compose.yaml").chmod(0o644)
    fixture["holder"] = fx.docker("create", "--name", "sl-holder", IMAGE, *SLOW)
    engine_id = fx.docker("info", "--format", "{{.ID}}")
    value = catalog_value(engine_id)
    data = json.dumps(value).encode()
    check(len(data) <= 16 * 1024, "fixture catalog too large")
    temporary = CATALOG.with_suffix(".new")
    temporary.write_bytes(data)
    temporary.chmod(0o644)
    temporary.replace(CATALOG)
    fx.run("systemctl", "start", "limeos-storage-reader")
    fixture["managed"] = [fixture["holder"]]
    fixture["policy"] = write_policy(
        allow_restart=True,
        allow_start=True,
        allow_stop=True,
        allow_stack=True,
        managed_stacks=list(PROJECTS),
        managed_containers=fixture["managed"],
    )
    provision("sl1")
    # Gate: abort the whole run early (with the last inventory) if the shared Stack inventory never converges.
    refresh_cohort("sl1", fixture["stacks"]["sl1"], timeout=75)
    return engine_id


# ---------------------------------------------------------------- cases
def effect_counts(since, ids, action):
    seen = effects(since, ids)
    return seen, [seen[i].get(action, 0) for i in ids]


@case("1 stop/start/restart: plan, approve, queue, verify, new StartedAt")
def lifecycle():
    ids = members("sl1")
    isolate("sl1", True)
    detail = {}
    for action, key in (("stop", "sl1-stop"), ("start", "sl1-start"), ("restart", "sl1-restart")):
        before = {i: started_at(i) for i in ids}
        proposal, approval = approved_plan("sl1", action, ids)
        check(proposal["plan"]["catalog_stack"] == "cat-sl1", "catalog stack not recorded", plan=proposal["plan"])
        check(
            [m["resource"] for m in proposal["plan"]["expected"]["members"]] == ["container:" + i for i in ids],
            "plan members are not the ID-ordered full-ID set",
        )
        event_mark = mark()
        status, job, body = stack_queue(proposal, approval, key)
        check(status == 202, "queue refused", status=status, job=job)
        check(wait_job(job["id"]) == "succeeded", f"{action} did not succeed", progress=progress(job["id"]))
        value = progress(job["id"])
        check(value["receipt"]["state"] == "verified", "aggregate not verified", receipt=value["receipt"])
        check(all(m["state"] == "verified" for m in value["receipt"]["members"]), "member not verified")
        after = {i: fx.inspect(i)["State"] for i in ids}
        if action == "stop":
            check(all(not s["Running"] for s in after.values()), "member still running")
        else:
            check(all(s["Running"] for s in after.values()), "member not running")
            check(all(after[i]["StartedAt"] != before[i] for i in ids), "StartedAt unchanged", before=before)
        seen, counts = effect_counts(event_mark, ids, action)
        check(all(c == 1 for c in counts), "not exactly one Engine effect per member", events=seen)
        check(value["events"][-1]["event"].get("state") == "succeeded", "progress did not end succeeded")
        check(locks_consistent() == [], "locks left after success")
        order_seen = []
        for line in EVENTS.read_text().splitlines()[event_mark:]:
            event = json.loads(line)
            if (event.get("Action") or "").split(":")[0] == action and event["id"] not in order_seen:
                order_seen.append(event["id"])
        check(order_seen == ids, "members were not acted on in ID order", order=order_seen)
        detail[action] = {"job": job["id"], "events": seen, "receipt": value["receipt"]["state"]}
        fixture["sl1_body"] = body
        fixture["sl1_job"] = job["id"]
    return detail


@case("2 idempotent replay returns same job with no Engine effect; second key conflicts")
def replay():
    ids = members("sl1")
    body = fixture["sl1_body"]
    event_mark = mark()
    status, _, job = fx.http(f"{STACK}/jobs", "POST", body, session["cookie"], session["csrf"])
    check(status == 202 and job["id"] == fixture["sl1_job"], "replay did not return the same job", status=status, job=job)
    second = dict(body, idempotency_key="sl1-other-key")
    status2, _, error = fx.http(f"{STACK}/jobs", "POST", second, session["cookie"], session["csrf"])
    check(status2 == 409, "consumed approval with a second key was not a conflict", status=status2, body=error)
    time.sleep(3)
    seen = effects(event_mark, ids)
    check(all(not counter for counter in seen.values()), "replay caused Engine events", events=seen)
    check(job_state(fixture["sl1_job"]) == "succeeded", "original job changed")
    with sqlite3.connect(fx.CORE_DB) as db:
        count = db.execute("SELECT COUNT(*) FROM jobs WHERE idempotency IN ('sl1-restart','sl1-other-key')").fetchone()[0]
    check(count == 1, "second key created a job", count=count)
    return {"replay_status": status, "second_key_status": status2, "events": seen}


@case("3 member drift between plan and queue is refused with no effect", retire=("sl2",))
def drift():
    ids = provision("sl2")
    isolate("sl2", True)
    proposal, approval = approved_plan("sl2", "stop", ids)
    fx.docker("stop", "-t", "3", ids[1])  # by hand, bypassing LimeOS
    event_mark = mark()
    status, job, _ = stack_queue(proposal, approval, "sl2-drift")
    check(status == 409, "drifted plan was not refused", status=status, job=job)
    time.sleep(3)
    check(running(ids[0]) and running(ids[2]), "untouched members were acted on")
    seen = effects(event_mark, ids)
    check(all(not counter for counter in seen.values()), "drift refusal caused Engine events", events=seen)
    with sqlite3.connect(fx.CORE_DB) as db:
        check(db.execute("SELECT COUNT(*) FROM jobs WHERE idempotency='sl2-drift'").fetchone()[0] == 0, "job created")
    check(locks_consistent() == [], "locks present")
    return {"queue_status": status, "refusal": job}


@case("3b drift after queue but before claim becomes precondition_changed with no effect", retire=("sl8",))
def drift_after_queue():
    ids = provision("sl8")
    isolate("sl8", True)
    holder = fixture["holder"]
    fx.docker("start", holder)
    proposal, approval = approved_plan("sl8", "stop", ids)
    held_plan, held_approval = container_plan(holder)
    event_mark = mark()
    status, held_job = container_queue(held_plan, held_approval, "sl8-holder")
    check(status == 202, "holder job refused", status=status)
    wait_event(holder, "kill", event_mark)  # the executor is now inside the 10 s Engine stop grace
    status, job, _ = stack_queue(proposal, approval, "sl8-queued")
    check(status == 202, "stack job not queued while the dispatcher is busy", status=status, job=job)
    fx.docker("stop", "-t", "1", ids[2])  # drift while the stack job waits
    drift_mark = mark()
    check(wait_job(held_job["id"], timeout=120) == "succeeded", "holder did not finish")
    check(wait_job(job["id"], timeout=60) == "precondition_changed", "drifted queued job was not refused", state=job_state(job["id"]))
    time.sleep(2)
    check(running(ids[0]) and running(ids[1]), "undrifted members were acted on")
    seen = effects(drift_mark, ids)
    check(all(not c.get("stop") for c in seen.values()), "stop effect despite drift", events=seen)
    check(locks_consistent() == [], "locks left")
    return {"final_state": "precondition_changed", "events_after_drift": seen}


@case("4a Core SIGKILL immediately after queueing: no member acted on twice", retire=("sl3",))
def core_kill_after_queue():
    ids = provision("sl3")
    isolate("sl3", True)
    proposal, approval = approved_plan("sl3", "restart", ids)
    event_mark = mark()
    status, job, _ = stack_queue(proposal, approval, "sl3-core-kill")
    check(status == 202, "queue refused", status=status)
    pid = kill_unit("limeos-core")
    wait_core()
    state = wait_job(job["id"], timeout=120)
    seen, counts = effect_counts(event_mark, ids, "restart")
    check(all(c <= 1 for c in counts), "member restarted more than once", events=seen)
    if state == "succeeded":
        check(all(c == 1 for c in counts), "succeeded but a member was not restarted", events=seen)
    elif state == "precondition_changed":
        check(all(c == 0 for c in counts), "released as unsent but effects occurred", events=seen)
    time.sleep(35)  # a full recovery interval: nothing may be replayed
    seen2, counts2 = effect_counts(event_mark, ids, "restart")
    check(counts2 == counts, "late replay after recovery", before=seen, after=seen2)
    locks_consistent()
    return {"killed_core_pid": pid, "state": state, "events": seen2, "receipts": receipts(job["id"])}


@case("4a' Core SIGKILL while executor is mid-restart: recovered by receipt, one effect each", retire=("sl4",))
def core_kill_during():
    ids = provision("sl4", slow=True)
    isolate("sl4", True)
    proposal, approval = approved_plan("sl4", "restart", ids)
    event_mark = mark()
    status, job, _ = stack_queue(proposal, approval, "sl4-core-during")
    check(status == 202, "queue refused", status=status)
    wait_event(ids[1], "kill", event_mark, timeout=90)  # executor is inside the second member's stop grace
    pid = kill_unit("limeos-core")
    wait_core()
    state = wait_job(job["id"], timeout=180)
    time.sleep(35)
    seen, counts = effect_counts(event_mark, ids, "restart")
    check(all(c == 1 for c in counts), "not exactly one restart per member", events=seen, state=state)
    state = job_state(job["id"])
    check(state == "succeeded", "executor completed but job is not succeeded", state=state, receipts=receipts(job["id"]))
    locks_consistent()
    return {"killed_core_pid": pid, "state": state, "events": seen, "receipts": receipts(job["id"])}


@case("4b executor SIGKILL during a multi-member restart: defined state, no member twice")
def executor_kill():
    ids = provision("sl5", slow=True)
    isolate("sl5", True)
    proposal, approval = approved_plan("sl5", "restart", ids)
    event_mark = mark()
    status, job, _ = stack_queue(proposal, approval, "sl5-exec-kill")
    check(status == 202, "queue refused", status=status)
    wait_event(ids[1], "kill", event_mark, timeout=90)  # first member done, second inside its stop grace
    pid = kill_unit("limeos-containerd")
    fx.wait_container_ready(timeout=45)
    state = wait_job(job["id"], timeout=150)
    time.sleep(40)  # span more than one recovery interval: no replay may follow
    seen, counts = effect_counts(event_mark, ids, "restart")
    check(all(c <= 1 for c in counts), "member restarted twice", events=seen)
    check(counts[2] == 0, "third member acted on after the executor died", events=seen)
    state_after = job_state(job["id"])
    check(state_after in ("needs_intervention", "failed"), "undefined or success state", state=state_after)
    rows = locks_consistent()
    fixture["sl5_job"] = job["id"]
    return {
        "killed_executor_pid": pid,
        "state": state_after,
        "events": seen,
        "locks": rows,
        "progress": progress(job["id"]),
        "receipts": receipts(job["id"]),
    }


@case("5a single-container job on a member is excluded while a stack job holds it", retire=("sl5",))
def locks_stack_holds():
    ids = members("sl5")
    isolate("sl5", None)
    state = job_state(fixture["sl5_job"])
    held = [row for row in locks() if row[1] == fixture["sl5_job"]]
    check(state == "needs_intervention" and held, "precondition: stack job must hold locks (case 4b)", state=state, held=held)
    target = ids[0]
    proposal, approval = container_plan(target)
    event_mark = mark()
    status, job = container_queue(proposal, approval, "sl5-single")
    check(status in (202, 409), "unexpected queue status", status=status, job=job)
    if status == 202:
        time.sleep(20)
        check(job_state(job["id"]) == "queued", "member job ran under a stack lock", state=job_state(job["id"]))
        seen = effects(event_mark, ids)
        check(all(not c for c in seen.values()), "member job produced Engine events", events=seen)
        cancel = http(f"{CONTAINER}/jobs/{job['id']}/cancel", "POST")[0]
        check(cancel == 204, "queued member job not cancelable", status=cancel)
        check(job_state(job["id"]) == "canceled", "not canceled")
    return {"queue_status": status, "stack_locks": held}


@case("5b stack job is excluded while a single-container job holds a member", retire=("sl6",))
def locks_container_holds():
    ids = provision("sl6", slow=True)
    isolate("sl6", True)
    member = ids[1]
    refresh_cohort("sl6", ids)
    proposal, approval = container_plan(member)
    event_mark = mark()
    status, held = container_queue(proposal, approval, "sl6-single")
    check(status == 202, "member job refused", status=status)
    wait_event(member, "kill", event_mark, timeout=60)
    kill_unit("limeos-containerd")
    fx.wait_container_ready(timeout=45)
    check(wait_job(held["id"], {"needs_intervention", "succeeded"}, 90) == "needs_intervention", "holder not held")
    time.sleep(12)  # the Engine finishes the already accepted restart
    refresh_cohort("sl6", ids)
    plan, token = approved_plan("sl6", "restart", ids)
    event_mark = mark()
    status, job, _ = stack_queue(plan, token, "sl6-stack")
    check(status == 202, "stack job refused at queue time", status=status, job=job)
    time.sleep(20)
    check(job_state(job["id"]) == "queued", "stack job ran despite a held member", state=job_state(job["id"]))
    seen = effects(event_mark, ids)
    check(all(not c for c in seen.values()), "excluded stack job produced Engine events", events=seen)
    cancel = http(f"{STACK}/jobs/{job['id']}/cancel", "POST")[0]
    check(cancel == 204 and job_state(job["id"]) == "canceled", "queued stack job not canceled", status=cancel)
    return {"held_member_job": held["id"], "locks": locks()}


@case("6 shadow package, task token and under-privileged human cannot execute a stack action")
def refusals():
    detail = {}
    isolate("sl1", True)
    # (a) Read-only shadow executor refuses the stack ceiling at startup.
    version = session["version"]
    fx.run("apt-get", "install", "-y", "limeos-shadow=" + version)
    shadow_policy = Path("/etc/limeos-shadow/system-policy/container.json")
    original = json.loads(shadow_policy.read_text())
    shadow_policy.write_text(
        json.dumps(
            {**original, "allow_restart": False, "allow_start": False, "allow_stop": False, "allow_stack": True,
             "managed_stacks": ["sl1"], "managed_containers": members("sl1")}
        )
    )
    fx.run("systemctl", "reset-failed", "limeos-shadow-containerd")
    fx.run("systemctl", "restart", "limeos-shadow-containerd", check=False)
    fx.eventually(
        lambda: fx.run("systemctl", "show", "limeos-shadow-containerd", "-p", "Result", "--value").stdout.strip() == "exit-code",
        timeout=15,
    )
    detail["shadow_executor_result"] = "exit-code"
    shadow_cookie, shadow_csrf, _ = fx.enroll("limeos-shadow", 8004)
    status, _, body = fx.http(
        f"{STACK}/plans", "POST", {"project": "sl1", "operation": "stop"}, shadow_cookie, shadow_csrf,
        origin="https://limeos-shadow.localhost:8444", port=8004,
    )
    check(status != 201, "shadow package produced a stack plan", status=status, body=body)
    detail["shadow_plan_status"] = status
    fx.run("apt-get", "remove", "-y", "limeos-shadow")
    event_mark = mark()
    # (b) Task token protocol has no stack operation.
    token = "9" * 64
    with sqlite3.connect(fx.CORE_DB) as db:
        generation = db.execute("SELECT generation FROM meta").fetchone()[0]
        scopes = [{"operation": "container_manage", "resource": "stack:sl1"}] + [
            {"operation": "container_manage", "resource": "container:" + i} for i in members("sl1")
        ]
        db.execute(
            "INSERT INTO task_tokens VALUES(?,?,?,?,?,?,?,?,0)",
            (hashlib.sha256(token.encode()).hexdigest(), session["principal"]["id"], pwd.getpwnam("limeos-assistant").pw_uid,
             "stack-task", json.dumps(scopes), session["principal"]["grant_revision"], generation, int(time.time()) + 3600),
        )
    for request in ("propose_stack", "queue_stack", "approve_stack"):
        reply = fx.rpc_as(
            "limeos-assistant",
            {"request": request, "version": 1, "token": token, "task": "stack-task", "project": "sl1", "operation": "stop"},
        )
        check(reply.get("code") == "invalid_input", "task token reached a stack operation", reply=reply)
        detail["task_" + request] = reply.get("code")
    # (c) A human whose grants do not reach every member cannot plan.
    scoped_id, session_secret = "c" * 64, "d" * 64
    digest = lambda value: hashlib.sha256(value.encode()).hexdigest()  # noqa: E731
    scoped_csrf = digest("csrf:" + session_secret)
    with sqlite3.connect(fx.CORE_DB) as db:
        hashed = db.execute("SELECT password_hash FROM users WHERE id=?", (session["principal"]["id"],)).fetchone()[0]
        db.execute("INSERT INTO users VALUES(?,?,?,?,1)", (scoped_id, "scoped-sl", hashed, '"operator"'))
        db.execute("INSERT INTO grants VALUES(?,?,?)", (scoped_id, '"container_manage"', "container:" + members("sl1")[0]))
        db.execute("INSERT INTO sessions VALUES(?,?,?,?,?,0)",
                   (digest(session_secret), scoped_id, digest(scoped_csrf), 1, int(time.time()) + 3600))
    refresh_cohort("sl1", members("sl1"))
    status, _, body = fx.http(
        f"{STACK}/plans", "POST", {"project": "sl1", "operation": "stop"}, "__Host-limeos=" + session_secret, scoped_csrf
    )
    check(status == 403, "under-privileged human was not forbidden", status=status, body=body)
    detail["scoped_human_plan_status"] = status
    # (d) Unauthenticated, wrong method and unknown body fields.
    check(fx.http(f"{STACK}/plans", "POST", {"project": "sl1", "operation": "stop"})[0] == 401, "anonymous plan")
    check(http(f"{STACK}/plans")[0] == 405, "GET plan accepted")
    check(http(f"{STACK}/plans", "POST", {"project": "sl1", "operation": "stop", "container": "x"})[0] in (400, 422), "unknown field")
    seen = effects(event_mark, members("sl1"))
    check(all(not c for c in seen.values()), "refusals caused Engine events", events=seen)
    return detail


@case("7 member that cannot start: per-member outcomes, never success, no second attempt", retire=("sl7",))
def failing_member():
    ids = provision("sl7", bad_middle=True)
    isolate("sl7", False)
    proposal, approval = approved_plan("sl7", "start", ids)
    event_mark = mark()
    status, job, _ = stack_queue(proposal, approval, "sl7-start")
    check(status == 202, "queue refused", status=status, job=job)
    state = wait_job(job["id"], timeout=90)
    time.sleep(3)
    first = {i: engine_attempt_state(i) for i in ids}
    first_events = effects(event_mark, ids)
    time.sleep(40)  # more than one recovery interval: no further attempt may occur
    second = {i: engine_attempt_state(i) for i in ids}
    value = progress(job["id"])
    state_after = job_state(job["id"])
    seen = effects(event_mark, ids)
    check(state != "succeeded" and state_after != "succeeded", "failure reported as success", state=state_after)
    check(state_after in ("needs_intervention", "failed", "outcome_unknown"), "undefined terminal state", state=state_after)
    receipt = value["receipt"]
    check(receipt is not None and receipt["state"] != "verified", "receipt claims success", receipt=receipt)
    outcomes = {m["resource"]: m["state"] for m in receipt["members"]}
    errors = {m["resource"]: m.get("error") for m in receipt["members"]}
    check(set(outcomes) == {"container:" + i for i in ids}, "per-member outcomes missing", outcomes=outcomes)
    check(outcomes["container:" + ids[0]] == "verified", "first member should be verified", outcomes=outcomes)
    check(outcomes["container:" + ids[1]] != "verified", "failed member reported verified", outcomes=outcomes)
    check(outcomes["container:" + ids[2]] == "precondition_changed", "later member was not left unsent", outcomes=outcomes)
    check(first == second, "Engine state of a member changed after the job ended (second attempt?)", first=first, second=second)
    check(first_events == seen, "Engine events appeared after the job ended", first=first_events, second=seen)
    check(seen[ids[0]].get("start") == 1 and not seen[ids[2]].get("start"), "start events differ", events=seen)
    check(second[ids[1]]["error"] and not running(ids[1]) and not running(ids[2]), "Engine did not record the failed start", state=second[ids[1]])
    return {"state": state_after, "outcomes": outcomes, "member_errors": errors, "engine_state": second, "events": seen, "locks": locks(), "receipts": receipts(job["id"])}


# ---------------------------------------------------------------- driver
def memory_facts():
    facts = {}
    for unit in ("limeos-core", "limeos-containerd", "limeos-storage-reader"):
        try:
            pid = unit_pid(unit)
            fields = {
                line.split(":")[0]: line.split(":")[1].strip()
                for line in Path(f"/proc/{pid}/smaps_rollup").read_text().splitlines()
                if ":" in line
            }
            facts[unit] = {"pss_kib": int(fields["Pss"].split()[0]), "swap_kib": int(fields["Swap"].split()[0])}
        except (OSError, ValueError, KeyError):
            facts[unit] = None
    meminfo = {l.split(":")[0]: int(l.split()[1]) for l in Path("/proc/meminfo").read_text().splitlines()}
    facts["guest_mem_available_kib"] = meminfo["MemAvailable"]
    facts["guest_mem_total_kib"] = meminfo["MemTotal"]
    return facts


def main(package_version="0.4.15+stacklc.1"):
    if os.getuid() != 0 or socket.gethostname() != "limeos-p01-test":
        raise SystemExit("Disposable guest required")
    repository, output = map(Path, sys.argv[1:])
    session["version"] = package_version
    started = time.time()
    engine_id = setup(repository, package_version)
    facts = {"engine_id": engine_id, "engine_fixture": {k: v for k, v in fixture["engine"].items() if k in ("peer", "unit", "server_version", "docker_package_version")}, "docker_version": fx.docker("version", "--format", "{{.Server.Version}}"),
             "memory_before": memory_facts(), "setup_seconds": round(time.time() - started, 1)}
    for run in order:
        run()
    facts["memory_after"] = memory_facts()
    facts["binary_sha256"] = {
        name: hashlib.sha256((Path("/usr/lib/limeos") / name).read_bytes()).hexdigest()
        for name in ("limeos-core", "limeos-executor", "limeosctl", "limeos-password-worker", "limeos-stack-read-worker")
    }
    facts["final_locks"] = locks()
    facts["docker_events_total"] = len(EVENTS.read_text().splitlines())
    document = {"summary": "pass" if all(r["status"] == "pass" for r in results.values()) else "fail",
                "package_version": package_version, "cases": results, "facts": facts}
    encoded = json.dumps(document, indent=2, default=str) + "\n"
    output.write_text(encoded)
    print("STACK_RESULT_BEGIN\n" + encoded + "STACK_RESULT_END", flush=True)
    if document["summary"] != "pass":
        raise SystemExit(1)


if __name__ == "__main__":
    main()
