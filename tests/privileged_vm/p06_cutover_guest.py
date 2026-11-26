#!/usr/bin/env python3
"""Basic-cutover rehearsal; exclusively inside the disposable Debian VM that imitates wybie.

Runs the real scripts/cutover/{preflight,activate,return}.sh, from a directory built by
scripts/cutover/bundle.sh and copied to /root/limeos-cutover as on the host, and `limeosctl migrate` against:
  * an imitated frozen app with wybie's units (26 Nov host facts): pi-health.service (HTTP on 0.0.0.0:8002 as
    holly:pihealth with StateDirectory=limeos, StateDirectoryMode=0750, ConfigurationDirectory, LogsDirectory,
    UMask=0027), pihealth-helper.service (PartOf), the package-reconcile timer and the metrics-collector timer
    whose service also runs as holly:pihealth with StateDirectory=limeos and writes /var/lib/limeos/metrics.sqlite3;
    so systemd owns /var/lib/limeos for them (holly:pihealth 0750) and chowns it at their starts;
    /etc/limeos/credentials.env with a real Werkzeug scrypt and PBKDF2 hash, state JSON/SQLite files named as on
    wybie, the agent supervisor's Python virtual environment (deeper than the scan's depth bound);
  * bystander units limeops-*, limeos-agent, flask-shutdown that must never change; limeops-worker, as its own
    account limeops-action-worker in group pihealth (as wybie's limeops-* accounts), keeps writing
    /var/lib/limeos/agent-actions/supervision.sqlite3 (agent-actions holly:pihealth 2770) while the cutover runs; docker-compose-start.service is failed since boot, as on wybie;
  * the packaged Docker Engine (socket-activated, as on a stock host), eleven real containers created by
    `holly` (compose project `media` with four running members and one stopped, project `mattermost` under
    /opt/stacks with three stopped, project `docker` outside /opt/stacks with one stopped, plus two standalone),
    /opt/stacks owned by holly;
  * Caddy from Debian, installed but inactive, serving the cutover site on 8002 with its internal CA;
  * a Jellyfin stand-in: the `jellyfin` container serves an idle `/Sessions` list on 8096 (busybox httpd).
The scripts are invoked as docs/p06-cutover-runbook.md says an operator does: the state list comes from
the runbook's own commands (read from the runbook and run as written), the Jellyfin key from a root-only
file, every required option is passed. Later cases cover what only a real host can show: return.sh
--force through the real cgroup.kill on systemd 252 (also once the scope's cgroup is gone), return.lock
refusing a second return.sh, measure.sh over the real /proc and cgroup tree, the unit hardening and
core's notify/watchdog, and the package maintainer scripts on real dpkg (remove, second remove,
reinstall, upgrade to a second build labelled 0.4.17+cutover.2).
Each case records pass/fail and continues. The full result is printed between CUTOVER_RESULT markers.
"""

import glob
import hashlib
import importlib.util
import json
import os
import pwd
import re
import shutil
import signal
import socket
import sqlite3
import ssl
import subprocess
import sys
import tarfile
import threading
import time
import traceback
from http import client as http_client
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path

import p03_guest as fx

HOST = "wybie.lan"
PORT = 8002
SITE = f"https://{HOST}:{PORT}"
SRC = Path("/root/limeos-src")
# The operator copies the directory bundle.sh builds (runbook step 1a); every script runs from there.
CUTOVER = Path("/root/limeos-cutover")
LOGS = Path("/root/cutover-logs")
CA = Path("/var/lib/caddy/.local/share/caddy/pki/authorities/local/root.crt")
CORE_DB = Path(fx.CORE_DB)
POLICY = Path("/etc/limeos/system-policy/container.json")
CATALOG = Path("/etc/limeos/management-catalog.json")
IMAGE = "limeos-cutover:local"
PASSWORD = "legacy-password"
FROZEN_UNITS = ["pihealth-helper.service", "pi-health.service", "limeos-metrics-collector.timer", "limeos-package-reconcile.timer"]
BYSTANDERS = ["limeops-actuatord.service", "limeops-worker.service", "limeos-agent.service", "flask-shutdown.service"]
RUST_UNITS = ["limeos-storaged.service", "limeos-containerd.service", "limeos-core.service"]
RUNNING = ["jellyfin", "radarr", "sonarr", "transmission", "watchtower", "vpn"]
MATTERMOST = ["limeos-mattermost", "limeos-mattermost-db", "limeos-alertd"]
EXITED = ["airsonic-advanced", *MATTERMOST, "airsonic-old"]
MEDIA_RUNNING = ["jellyfin", "radarr", "sonarr", "transmission"]
METRICS_DB = Path("/var/lib/limeos/metrics.sqlite3")
SUPERVISION_DB = Path("/var/lib/limeos/agent-actions/supervision.sqlite3")
VENV = Path("/var/lib/limeos/agent-supervisor/venv")
CLAIMANTS = ["pi-health.service", "limeos-metrics-collector.service"]
GUARD = "zz-limeos-cutover-guard.conf"
# LimeOS's trees inside the shared state directory and their accounts.
PACKAGE_TREES = {"/var/lib/limeos/core": "limeos-core", "/var/lib/limeos/assistant": "limeos-assistant", "/var/lib/limeos/executors/container": "limeos-containerd"}
RUNBOOK = SRC / "docs/p06-cutover-runbook.md"
# The runbook's file names (step 1 and step 2).
STATE_LIST = Path("/root/cutover-state-paths.txt")
LIVE_LIST = Path("/root/cutover-live-paths.txt")
# What the live list must cover here (as on wybie): the limeops-* homes and the file the "host backup" rewrites.
LIVE_PATHS = ("/var/lib/limeos/agent-actions/", "/var/lib/limeos/agent-supervisor/")
KEY_FILE = Path("/root/jellyfin.key")
JELLYFIN_KEY = "0123456789abcdef0123456789abcdef"
UPGRADE_VERSION = "0.4.17+cutover.2"
# Every unit the package ships (/lib/systemd/system); the ceilings come from scripts/check_unit_exposure.py.
SHIPPED_UNITS = [
    "limeos-core.service", "limeos-containerd.service", "limeos-storaged.service", "limeos-storage-reader.service",
    "limeos-storage-targets.service", "limeos-storage-ready.service", "limeos-storage-mounts.service",
]
# activate.sh's package step is held here (a dpkg pre-invoke hook) so a forced return meets a live activation.
HOLD_CONFIG = Path("/etc/dpkg/dpkg.cfg.d/zz-rehearsal-hold")
HOLD_SCRIPT = Path("/root/rehearsal-hold.sh")
HOLD_MARK = Path("/run/rehearsal-hold-reached")

results = {}
order = []
state = {"frozen_hashes": {}, "logs": {}, "notes": [], "session": {}, "timings": {}}


# ---------------------------------------------------------------- helpers
def problems(out):
    """The lines a script prints for what went wrong, kept whole however long the transcript is."""
    return [line for line in out.splitlines() if line.startswith(("FAIL", "ERROR", "WARN", "ROLLBACK", "REFUSED", "TIMEOUT"))]


def check(condition, message, **facts):
    if not condition:
        raise AssertionError(message + (" " + json.dumps(facts, default=str)[:2500] if facts else ""))


def case(name):
    def decorate(function):
        def run():
            started = time.monotonic()
            try:
                detail = function() or {}
                results[name] = {"status": "pass", "detail": detail}
                fx.passed(name)
            except BaseException as error:  # noqa: BLE001 - recorded, never hidden
                results[name] = {"status": "fail", "error": repr(error)[:3500], "trace": traceback.format_exc()[-3500:]}
                print("FAIL " + name + ": " + repr(error)[:2500], flush=True)
            results[name]["seconds"] = round(time.monotonic() - started, 1)

        order.append(run)
        return function

    return decorate


def sh(command, check_rc=True, timeout=300, log=None, env=None):
    """Run a shell command; returns (rc, combined output). `log` keeps the full text as raw evidence."""
    merged = {**os.environ, **(env or {})}
    result = subprocess.run(["bash", "-c", command], capture_output=True, text=True, timeout=timeout, env=merged)
    out = result.stdout + result.stderr
    if log:
        shown = out if len(out) <= 40000 else out[:10000] + "\n[... truncated ...]\n" + out[-30000:]
        state["logs"][log] = f"$ {command}\n{shown}\n[exit {result.returncode}]\n"
    if check_rc and result.returncode:
        raise AssertionError(f"command failed ({result.returncode}): {command}\n{out[-2500:]}")
    return result.returncode, out


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def unit(name, *fields):
    wanted = fields or ("ActiveState", "UnitFileState", "MainPID", "InvocationID", "ActiveEnterTimestampMonotonic")
    out = sh(f"systemctl show {name} " + " ".join(f"-p {f}" for f in wanted))[1]
    return dict(line.split("=", 1) for line in out.splitlines() if "=" in line)


def active(name):
    return sh(f"systemctl is-active {name}", check_rc=False)[1].strip()


def enabled(name):
    return sh(f"systemctl is-enabled {name}", check_rc=False)[1].strip()


def containers():
    names = sh("docker ps -a --format '{{.Names}}'")[1].split()
    result = {}
    for name in sorted(names):
        d = json.loads(sh(f"docker inspect {name}")[1])[0]
        s = d["State"]
        result[name] = {
            "id": d["Id"],
            "started": s["StartedAt"],
            "finished": s["FinishedAt"],
            "pid": s["Pid"],
            "running": s["Running"],
            "restarts": d["RestartCount"],
        }
    return result


def bystanders():
    return {u: unit(u) for u in BYSTANDERS}


def hash_tree():
    """sha256 of every frozen-app file recorded at fixture time (SQLite and live state excluded; see sqlite_facts)."""
    return {p: sha(p) for p in state["frozen_files"] if Path(p).exists()}


def owner_mode(path):
    st = os.lstat(path)
    return st.st_uid, st.st_gid, st.st_mode & 0o7777


def foreign_entries(tree, account):
    """Entries under TREE that ACCOUNT does not own (systemd's recursive chown of 26 Nov leaves them holly's)."""
    uid = pwd.getpwnam(account).pw_uid
    found = []
    for base, dirs, names in os.walk(tree):
        found += [os.path.join(base, n) for n in [*dirs, *names] if os.lstat(os.path.join(base, n)).st_uid != uid]
    if os.lstat(tree).st_uid != uid:
        found.append(tree)
    return found


def expect_state_dir_frozen(label):
    """/var/lib/limeos is the frozen app's (holly:pihealth 0750), LimeOS's trees inside it LimeOS's, no guard left."""
    holly = pwd.getpwnam("holly").pw_uid
    pihealth = int(sh("getent group pihealth | cut -d: -f3")[1].strip())
    check(owner_mode("/var/lib/limeos") == (holly, pihealth, 0o750), f"{label}: /var/lib/limeos is not holly:pihealth 0750", now=permitted("/var/lib/limeos"))
    for tree, account in PACKAGE_TREES.items():
        if Path(tree).exists():
            check(not foreign_entries(tree, account), f"{label}: {tree} has entries not owned by {account}", entries=foreign_entries(tree, account)[:10])
    for unit_name in CLAIMANTS:
        check(not Path(f"/etc/systemd/system/{unit_name}.d/{GUARD}").exists(), f"{label}: the cutover guard on {unit_name} is left behind")


def sqlite_facts():  # the frozen app's metrics database
    with sqlite3.connect(f"file:{METRICS_DB}?mode=ro", uri=True) as db:
        return {"rows": db.execute("select count(*) from samples").fetchone()[0], "integrity": db.execute("pragma integrity_check").fetchone()[0]}


def frozen_world():
    return {u: {"active": active(u), "enabled": enabled(u)} for u in FROZEN_UNITS}


def listeners():
    return sh("ss -H -ltnp")[1]


def plain_get(port, path="/"):
    connection = http_client.HTTPConnection("127.0.0.1", port, timeout=5)
    try:
        connection.request("GET", path)
        response = connection.getresponse()
        return response.status, response.read().decode(errors="replace")
    except OSError as error:
        return None, repr(error)
    finally:
        connection.close()


def https(path, method="GET", body=None, cookie=None, csrf=None, port=PORT, host=HOST):
    context = ssl.create_default_context(cafile=str(CA))
    connection = http_client.HTTPSConnection(host, port, context=context, timeout=15)
    headers = {"Origin": f"https://{host}:{port}", "Content-Type": "application/json"}
    if cookie:
        headers["Cookie"] = cookie
    if csrf:
        headers["X-CSRF-Token"] = csrf
    connection.request(method, path, json.dumps(body) if body is not None else None, headers)
    response = connection.getresponse()
    data = response.read()
    result = (response.status, {k.lower(): v for k, v in response.getheaders()}, json.loads(data) if data and data[:1] in b"{[" else data.decode(errors="replace"))
    connection.close()
    return result


def login(username, password=PASSWORD, via_https=True):
    if via_https:
        status, headers, view = https("/api/v1/auth/login", "POST", {"username": username, "password": password})
    else:
        status, headers, view = fx.http("/api/v1/auth/login", "POST", {"username": username, "password": password}, origin=SITE)
    return status, headers, view


def users():
    with sqlite3.connect(f"file:{CORE_DB}?mode=ro", uri=True) as db:
        return dict(db.execute("select username,password_hash from users").fetchall())


def run_script(name, *args, log, timeout=420, check_rc=False):
    command = " ".join([str(CUTOVER / name), *map(str, args)])
    started = time.monotonic()
    rc, out = sh(command, check_rc=check_rc, timeout=timeout, log=log)
    state["timings"][log] = round(time.monotonic() - started, 1)
    return rc, out


def lock_holders(path):
    """PIDs holding a flock on PATH, read from /proc/locks (never by taking the lock, which could race the holder)."""
    if not Path(path).exists():
        return []
    inode = os.stat(path).st_ino
    pids = []
    for line in Path("/proc/locks").read_text().splitlines():
        fields = [f for f in line.split() if f != "->"]
        if len(fields) >= 6 and fields[1] == "FLOCK" and fields[5].endswith(f":{inode}"):
            pids.append(int(fields[4]))
    return pids


def runbook_block(after):
    """The first ```sh block after the text AFTER in the runbook, as the operator would paste it."""
    text = RUNBOOK.read_text()
    found = re.search(r"```sh\n(.*?)```", text[text.index(after):], re.S)
    check(found, f"runbook has no sh block after {after!r}")
    return "\n".join(line.strip() for line in found.group(1).splitlines() if line.strip())


def permitted(path):
    return sh(f"stat -c '%U:%G %a' {path}")[1].strip()


# ---------------------------------------------------------------- fixture
APP = '''import http.server, json
class H(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        body = json.dumps({"app": "pi-health-imitation", "path": self.path}).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)
    def log_message(self, *args):
        pass
class S(http.server.ThreadingHTTPServer):
    allow_reuse_address = True
S(("0.0.0.0", 8002), H).serve_forever()
'''

COLLECTOR = '''import sqlite3, time
db = sqlite3.connect("/var/lib/limeos/metrics.sqlite3")
db.execute("create table if not exists samples (t integer, load real)")
db.execute("insert into samples values (?, ?)", (int(time.time()), 0.1))
db.commit()
'''

# wybie's limeops-* services write their supervision database continuously and do not stop for the cutover.
# Here one bystander does it every 2 s.
LIMEOPS_WRITER = '''import sqlite3, time
while True:
    db = sqlite3.connect("/var/lib/limeos/agent-actions/supervision.sqlite3")
    db.execute("create table if not exists beats (t real)")
    db.execute("insert into beats values (?)", (time.time(),))
    db.commit()
    db.close()
    time.sleep(2)
'''


def unit_file(name, text):
    Path(f"/etc/systemd/system/{name}").write_text(text)


def write_fixture(version_hashes):
    sh("useradd -m -s /bin/bash holly")
    sh("apt-get update -qq", timeout=300)
    sh("DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends docker.io busybox-static caddy sqlite3 iproute2 curl strace", timeout=900, log="apt-fixture")
    sh("usermod -aG docker holly")
    sh("groupadd --system pihealth")
    # wybie's limeops-* services run as their own accounts and reach agent-actions through group pihealth.
    sh("useradd --system --no-create-home --shell /usr/sbin/nologin limeops-action-worker && usermod -aG pihealth limeops-action-worker")
    sh("systemctl stop caddy; systemctl disable caddy", check_rc=False)
    Path("/etc/caddy/Caddyfile").write_text("import /etc/caddy/conf.d/*.caddy\n")
    with open("/etc/hosts", "a") as hosts:
        hosts.write(f"127.0.0.1 {HOST}\n")
    # Frozen app files (owned by holly), state files, credentials with real Werkzeug hashes.
    app = Path("/home/holly/pi-health")
    (app / "config/storage_plugins").mkdir(parents=True)
    (app / "app.py").write_text(APP)
    (app / "config/backup_config.json").write_text('{"enabled": false, "target": "/mnt/backup", "edited": true}\n')
    (app / "config/media_paths.json").write_text('{"movies": "/mnt/storage/movies", "tv": "/mnt/storage/tv"}\n')
    (app / "config/storage_plugins/example.json").write_text('{"plugin": "example"}\n')
    (app / "collector.py").write_text(COLLECTOR)
    sh("chown -R holly:holly /home/holly/pi-health")
    scrypt = next(e["hash"] for e in version_hashes if e["hash"].startswith("scrypt:"))
    pbkdf2 = next(e["hash"] for e in version_hashes if e["hash"].startswith("pbkdf2:"))
    Path("/etc/limeos").mkdir(exist_ok=True)
    Path("/etc/limeos/credentials.env").write_text(f"PIHEALTH_USERS=holly:{scrypt},bob:{pbkdf2}\nDISK_PATH=/mnt/storage\nDISK_PATH_2=/mnt/storage2\n")
    sh("chown root:holly /etc/limeos/credentials.env; chmod 0640 /etc/limeos/credentials.env")
    Path("/etc/limeos/agent-policy.json").write_text('{"allowed_units": ["pi-health", "pihealth-helper"]}\n')
    Path("/etc/limeos/policy-backups").mkdir()
    for n in range(5):
        Path(f"/etc/limeos/policy-backups/agent-policy-2026100{n}T010203Z.json").write_text(f'{{"backup": {n}}}\n')
    for d in ("agent-actions", "agent-supervisor", "integrations"):
        Path(f"/var/lib/limeos/{d}").mkdir(parents=True)
    for name in ("auto_update.json", "backup_config.json", "package-approvals.json", "pending-actions.json"):
        Path(f"/var/lib/limeos/{name}").write_text('{"frozen": true}\n')
    Path("/var/lib/limeos/integrations/jellyfin.json").write_text('{"api_key": "synthetic-not-real"}\n')
    for name in ("actions", "automation", "findings", "supervision"):
        with sqlite3.connect(f"/var/lib/limeos/agent-actions/{name}.sqlite3") as db:
            db.execute("create table t (a)")
            db.execute("insert into t values (1)")
    # Group pihealth may write there (setgid, as wybie's drwxrws---); systemd's chown keeps the modes.
    os.chmod("/var/lib/limeos/agent-actions", 0o2770)
    os.chmod(SUPERVISION_DB, 0o660)
    # The limeops accounts' homes hold skeleton files; the agent supervisor's virtualenv is 9 levels deep.
    for home in ("agent-actions", "agent-supervisor"):
        for skel in (".bashrc", ".profile", ".bash_logout"):
            Path(f"/var/lib/limeos/{home}/{skel}").write_text("# skeleton\n")
    deep = VENV / "lib/python3.11/site-packages/pip/_internal/cli/__pycache__/a/b/c/d"
    deep.mkdir(parents=True)
    (VENV / "pyvenv.cfg").write_text("home = /usr/bin\ninclude-system-site-packages = false\nversion = 3.11.2\n")
    (deep / "main.cpython-311.pyc").write_bytes(b"\x00pyc")
    (VENV / "lib/python3.11/site-packages/pip/__init__.py").write_text("__version__ = '23.0'\n")
    sh(f"python3 {app}/collector.py")  # creates metrics.sqlite3 with the first sample
    Path("/usr/local/lib/limeops-writer.py").write_text(LIMEOPS_WRITER)
    # Units, as `systemctl cat` showed them on wybie (26 Nov) where it matters: who owns /var/lib/limeos.
    unit_file("pi-health.service", (
        f"[Unit]\nDescription=Pi-Health (frozen app imitation)\nAfter=network.target docker.service\nWants=docker.service\n[Service]\nType=simple\n"
        f"User=holly\nGroup=pihealth\nWorkingDirectory={app}\nEnvironmentFile=-/etc/limeos/credentials.env\n"
        "Environment=LIMEOS_CONFIG_DIR=/etc/limeos\nEnvironment=LIMEOS_STATE_DIR=/var/lib/limeos\n"
        f"ExecStart=/usr/bin/python3 {app}/app.py\nRestart=on-failure\nRestartSec=3\nUMask=0027\n"
        "ConfigurationDirectory=limeos\nConfigurationDirectoryMode=0750\nStateDirectory=limeos\nStateDirectoryMode=0750\n"
        "LogsDirectory=limeos\nLogsDirectoryMode=0750\n[Install]\nWantedBy=multi-user.target\n"))
    unit_file("pihealth-helper.service", "[Unit]\nDescription=Pi-Health helper (root)\nPartOf=pi-health.service\n[Service]\nExecStart=/bin/sleep infinity\n[Install]\nWantedBy=multi-user.target\n")
    unit_file("limeos-metrics-collector.service", (
        f"[Unit]\nDescription=metrics collector\nAfter=local-fs.target\n[Service]\nType=oneshot\nUser=holly\nGroup=pihealth\n"
        f"WorkingDirectory={app}\nEnvironment=LIMEOS_STATE_DIR=/var/lib/limeos\nExecStart=/usr/bin/python3 {app}/collector.py\n"
        "NoNewPrivileges=true\nProtectSystem=strict\nProtectHome=read-only\nPrivateTmp=true\nReadWritePaths=/var/lib/limeos\n"
        "UMask=0027\nStateDirectory=limeos\nStateDirectoryMode=0750\n"))
    unit_file("limeos-metrics-collector.timer", "[Unit]\nDescription=metrics collector timer\n[Timer]\nOnBootSec=5s\nOnUnitActiveSec=20s\n[Install]\nWantedBy=timers.target\n")
    unit_file("limeos-package-reconcile.service", "[Unit]\nDescription=package reconcile\n[Service]\nType=oneshot\nExecStart=/bin/true\n")
    unit_file("limeos-package-reconcile.timer", "[Unit]\nDescription=package reconcile timer\n[Timer]\nOnCalendar=hourly\nPersistent=false\n[Install]\nWantedBy=timers.target\n")
    unit_file("limeops-actuatord.service", "[Unit]\nDescription=limeops actuatord\nAfter=pihealth-helper.service\n[Service]\nExecStart=/bin/sleep infinity\n[Install]\nWantedBy=multi-user.target\n")
    unit_file("limeops-worker.service", "[Unit]\nDescription=limeops worker (writes its supervision database)\n[Service]\nUser=limeops-action-worker\nGroup=limeops-action-worker\nUMask=0007\nRestart=on-failure\nReadWritePaths=/var/lib/limeos/agent-actions\nExecStart=/usr/bin/python3 /usr/local/lib/limeops-writer.py\n[Install]\nWantedBy=multi-user.target\n")
    for name in ("limeos-agent", "flask-shutdown"):
        unit_file(f"{name}.service", f"[Unit]\nDescription={name}\n[Service]\nExecStart=/bin/sleep infinity\n[Install]\nWantedBy=multi-user.target\n")
    # wybie's docker-compose-start.service has been failed since boot; nothing in the cutover may trip on it.
    unit_file("docker-compose-start.service", "[Unit]\nDescription=compose start (fails, as on wybie)\n[Service]\nType=oneshot\nExecStart=/bin/false\n[Install]\nWantedBy=multi-user.target\n")
    sh("systemctl daemon-reload")
    sh("systemctl enable --now pi-health.service pihealth-helper.service limeos-metrics-collector.timer limeos-package-reconcile.timer limeops-actuatord.service limeops-worker.service limeos-agent.service flask-shutdown.service", log="enable-fixture")
    sh("systemctl enable docker-compose-start.service; systemctl start docker-compose-start.service", check_rc=False)
    fx.eventually(lambda: plain_get(8002)[0] == 200, timeout=20)
    # Docker: image from static busybox; containers created by holly in the docker group.
    fx.eventually(lambda: sh("docker info", check_rc=False)[0] == 0, timeout=90)
    rootfs = Path("/root/cutover-rootfs.tar")
    sessions = Path("/root/jellyfin-www/Sessions")
    sessions.parent.mkdir()
    sessions.write_text("[]\n")  # Jellyfin's /Sessions with nobody playing
    with tarfile.open(rootfs, "w") as archive:
        archive.add("/bin/busybox", arcname="bin/busybox")
        # Docker runs a health check through /bin/sh (CMD-SHELL); case 24 gives containers one.
        shell = tarfile.TarInfo("bin/sh")
        shell.type = tarfile.SYMTYPE
        shell.linkname = "busybox"
        archive.addfile(shell)
        archive.add(str(sessions.parent), arcname="www")
    sh(f"docker import {rootfs} {IMAGE}")
    stack = Path("/opt/stacks/media")
    stack.mkdir(parents=True)
    (stack / "compose.yaml").write_text("services:\n  jellyfin:\n    image: x\n  radarr:\n    image: x\n  sonarr:\n    image: x\n  transmission:\n    image: x\n  airsonic-advanced:\n    image: x\n")
    Path("/opt/stacks/mattermost").mkdir()
    Path("/opt/stacks/mattermost/compose.yaml").write_text("services:\n  limeos-mattermost:\n    image: x\n  limeos-mattermost-db:\n    image: x\n  limeos-alertd:\n    image: x\n")
    sh("chown -R holly:holly /opt/stacks; chmod 0755 /opt/stacks /opt/stacks/media /opt/stacks/mattermost; chmod 0644 /opt/stacks/media/compose.yaml /opt/stacks/mattermost/compose.yaml")
    # wybie's one-container project `docker` lives outside /opt/stacks (propose-catalog refuses it).
    Path("/home/pi/docker").mkdir(parents=True)
    Path("/home/pi/docker/docker-compose.yml").write_text("services:\n  airsonic-old:\n    image: x\n")

    def create(name, project=None, extra="", directory=None):
        labels = ""
        if project:
            directory = directory or f"/opt/stacks/{project}"
            compose = "docker-compose.yml" if directory.startswith("/home/") else "compose.yaml"
            labels = (
                f"--label com.docker.compose.project={project} --label com.docker.compose.service={name} "
                f"--label com.docker.compose.project.working_dir={directory} "
                f"--label com.docker.compose.project.config_files={directory}/{compose} --label com.docker.compose.oneoff=False "
            )
        # jellyfin answers preflight's playback check on 8096 like the real one (an idle session list).
        serve = "/bin/busybox httpd -f -p 8096 -h /www & " if name == "jellyfin" else ""
        command = f"echo {name} ready; {serve}trap 'exit 0' TERM INT; while :; do /bin/busybox sleep 1; done"
        return sh(
            f"runuser -u holly -- docker run -d --name {name} --restart unless-stopped {labels}{extra} {IMAGE} /bin/busybox sh -c \"{command}\""
        )[1].strip()

    ids = {}
    for name in MEDIA_RUNNING:
        ids[name] = create(name, "media", "-p 8096:8096" if name == "jellyfin" else "")
    ids["airsonic-advanced"] = create("airsonic-advanced", "media")
    sh("runuser -u holly -- docker stop -t 1 airsonic-advanced")
    ids["watchtower"] = create("watchtower")
    ids["vpn"] = create("vpn")
    for name in MATTERMOST:
        ids[name] = create(name, "mattermost")
    ids["airsonic-old"] = create("airsonic-old", "docker", directory="/home/pi/docker")
    sh("runuser -u holly -- docker stop -t 1 " + " ".join([*MATTERMOST, "airsonic-old"]))
    state["ids"] = ids
    # No management catalog exists yet: `limeosctl migrate propose-catalog` generates it (case 4b).
    # Baseline of the frozen app's files (taken before any LimeOS package exists).
    files = []
    for root in ("/home/holly/pi-health", "/etc/limeos", "/var/lib/limeos", "/etc/caddy/Caddyfile"):
        if Path(root).is_file():
            files.append(root)
        for base, _, names in os.walk(root):
            # SQLite files and their journals, and the live state the bystander writes, are checked otherwise.
            files += [os.path.join(base, n) for n in names
                      if not n.endswith((".sqlite3", ".pyc", "-journal", "-wal", "-shm")) or base.startswith("/home/")]
    files += [f"/etc/systemd/system/{n}" for n in ("pi-health.service", "pihealth-helper.service", "limeos-metrics-collector.service", "limeos-metrics-collector.timer", "limeos-package-reconcile.service", "limeos-package-reconcile.timer", "limeops-actuatord.service", "limeops-worker.service", "limeos-agent.service", "flask-shutdown.service")]
    state["frozen_files"] = sorted(set(files))
    state["frozen_hashes"] = hash_tree()
    state["baseline_bystanders"] = bystanders()


def setup(repository, version):
    hashes = json.loads(Path("/root/werkzeug-hashes.json").read_text())
    write_fixture(hashes)
    sh("install -m 0644 " + str(repository / "limeos-archive-keyring.gpg") + " /usr/share/keyrings/limeos-test.gpg")
    Path("/etc/apt/sources.list.d/limeos-test.list").write_text(f"deb [signed-by=/usr/share/keyrings/limeos-test.gpg] file:{repository} stable main\n")
    sh("apt-get update -qq", timeout=300)
    # Runbook step 1a: the operator copies the directory bundle.sh builds and checks its SHA256SUMS there.
    shutil.rmtree(CUTOVER, ignore_errors=True)
    sh(f"{SRC}/scripts/cutover/bundle.sh /root/limeos-cutover-build && cp -a /root/limeos-cutover-build {CUTOVER} && cd {CUTOVER} && sha256sum -c SHA256SUMS", log="bundle")
    debs = glob.glob(str(repository / f"**/limeos_{version}_*.deb"), recursive=True)
    check(len(debs) == 1, "expected exactly one limeos .deb in the repository", debs=debs)
    state["deb"] = debs[0]
    state["version"] = version
    # The upgrade case: a second build of the same binaries with a higher version label.
    upgrades = glob.glob(str(repository / f"**/limeos_{UPGRADE_VERSION}_*.deb"), recursive=True)
    check(len(upgrades) == 1, "expected exactly one upgrade .deb in the repository", debs=upgrades)
    LOGS.mkdir(exist_ok=True)


# ---------------------------------------------------------------- cases
@case("0 fixture: frozen app serving 8002 as holly:pihealth owning /var/lib/limeos, bystanders writing, 6 running + 5 stopped containers, a failed unit, Caddy installed but inactive")
def fixture_check():
    status, body = plain_get(8002)
    check(status == 200 and "pi-health-imitation" in body, "frozen app not serving", status=status)
    owner = sh("ps -o user= -p $(systemctl show pi-health -p MainPID --value)")[1].strip()
    check(owner == "holly", "frozen app not running as holly", owner=owner)
    check("0.0.0.0:8002" in listeners(), "8002 not listening on 0.0.0.0")
    check(all(frozen_world()[u] == {"active": "active", "enabled": "enabled"} for u in FROZEN_UNITS), "frozen units not enabled+active", world=frozen_world())
    c = containers()
    check(sorted(n for n, v in c.items() if v["running"]) == sorted(RUNNING), "running container set differs", names=list(c))
    check(all(not c[n]["running"] for n in EXITED), "a stopped container is running", names=list(c))
    # systemd made /var/lib/limeos the app's at its first start (StateDirectory=limeos), recursively.
    holly, pihealth = pwd.getpwnam("holly").pw_uid, int(sh("getent group pihealth | cut -d: -f3")[1].strip())
    check(owner_mode("/var/lib/limeos") == (holly, pihealth, 0o750), "/var/lib/limeos is not holly:pihealth 0750", now=permitted("/var/lib/limeos"))
    check(os.lstat("/var/lib/limeos/auto_update.json").st_uid == holly, "systemd did not chown the state files to holly")
    check(active("docker-compose-start.service") == "failed", "docker-compose-start.service is not failed", state=active("docker-compose-start.service"))
    beats = sqlite3.connect(f"file:{SUPERVISION_DB}?mode=ro", uri=True).execute("select count(*) from beats").fetchone()[0]
    check(beats > 0, "the limeops writer is not writing")
    check("docker" in sh("id -nG holly")[1].split(), "holly not in the docker group")
    check(sh("runuser -u holly -- docker ps -q")[1].split(), "holly cannot use docker")
    check(permitted("/opt/stacks/media") == "holly:holly 755", "stacks root not owned by holly")
    check(active("caddy") == "inactive" and enabled("caddy") == "disabled", "caddy should be installed but inactive/disabled", caddy=(active("caddy"), enabled("caddy")))
    check(" 8003 " not in listeners() and ":80 " not in listeners(), "unexpected listeners")
    sessions = sh("curl -sS --max-time 5 http://127.0.0.1:8096/Sessions", check_rc=False)[1]
    check(sessions.strip() == "[]", "the Jellyfin stand-in does not answer /Sessions on 8096", body=sessions[:300])
    state["docker_info"] = sh("docker version --format '{{.Server.Version}}'")[1].strip()
    state["caddy_version"] = sh("caddy version")[1].strip()
    peer = sh("python3 - <<'E'\nimport socket,struct\ns=socket.socket(socket.AF_UNIX);s.connect('/var/run/docker.sock')\nprint(struct.unpack('3i', s.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, 12)))\nE")[1].strip()
    state["docker_socket_peer_pid_uid_gid"] = peer
    return {"docker": state["docker_info"], "caddy": state["caddy_version"], "docker_socket_peer(pid,uid,gid)": peer, "containers": {n: {"running": v["running"], "id": v["id"][:12]} for n, v in c.items()}, "bystanders": state["baseline_bystanders"]}


@case("1 package install (apt, pinned version) while the frozen app owns /var/lib/limeos: directory left as it is, LimeOS trees created, services neither enabled nor started; frozen app untouched")
def install_package():
    before_c = containers()
    before_b = bystanders()
    rc, out = sh(f"DEBIAN_FRONTEND=noninteractive apt-get install -y limeos={state['version']}", timeout=600, log="apt-install-limeos")
    check("/var/lib/limeos belongs to holly:pihealth (750)" in out and "neither enabled nor started" in out, "configure did not say it left LimeOS stopped", out=out[-1500:])
    time.sleep(3)
    check(all(active(u) in ("inactive", "failed") and enabled(u) == "disabled" for u in RUST_UNITS), "configure enabled or started a LimeOS service", units={u: (active(u), enabled(u)) for u in RUST_UNITS})
    expect_state_dir_frozen("after the package install")
    check(all(Path(t).is_dir() for t in PACKAGE_TREES), "configure did not create LimeOS's trees")
    check(plain_get(8002)[0] == 200, "frozen app disturbed by package install")
    check(containers() == before_c, "containers changed by package install")
    check(bystanders() == before_b, "bystanders changed by package install")
    check(hash_tree() == state["frozen_hashes"], "frozen files changed by package install")
    # A start of the frozen app now (its timer fires every 20 s too) changes nothing of LimeOS's.
    sh("systemctl restart pi-health.service")
    fx.eventually(lambda: plain_get(8002)[0] == 200, timeout=20)
    expect_state_dir_frozen("after the frozen app restarted with LimeOS installed")
    # The operator sets the origin (runbook decision 3); core reads it when activation starts it.
    config = json.loads(Path("/etc/limeos/core.json").read_text())
    config["origin"] = SITE
    Path("/etc/limeos/core.json").write_text(json.dumps(config, indent=2) + "\n")
    return {"version": state["version"], "configure_notice": [l for l in out.splitlines() if l.startswith("LimeOS:")]}


@case("2 migrate scan: names every state source, redacted, originals unchanged")
def scan():
    before = hash_tree()
    rc, out = sh("/usr/lib/limeos/limeosctl migrate scan --checkout /home/holly/pi-health", check_rc=False, log="migrate-scan")
    rc2, js = sh("/usr/lib/limeos/limeosctl migrate scan --checkout /home/holly/pi-health --json", check_rc=False, log="migrate-scan-json")
    check(rc == 0 and rc2 == 0, "scan failed or incomplete (wybie: the agent supervisor's virtualenv past the depth bound)", rc=rc, rc2=rc2, out=out[-1500:])
    report = json.loads(js[js.index("{"):])
    venv = [f for f in report["files"] if f["path"].startswith("agent-supervisor/venv")]
    check([(f["path"], f["kind"]) for f in venv] == [("agent-supervisor/venv", "virtualenv")] and report["complete"], "the virtualenv is not one entry", venv=venv[:5], problems=report["problems"][:5])
    for needle in ("credentials.env", "agent-policy.json", "auto_update.json", "package-approvals.json", "pending-actions.json", "jellyfin.json", "actions.sqlite3", "automation.sqlite3", "findings.sqlite3", "supervision.sqlite3", "metrics.sqlite3", "policy-backups", "compose.yaml", "backup_config.json", "media_paths.json", "storage_plugins"):
        check(needle in out, f"scan report does not name {needle}", out=out[:1500])
    credentials = Path("/etc/limeos/credentials.env").read_text()
    for secret in re.findall(r"(?:scrypt|pbkdf2)[:\w]*\$[^\s,]+", credentials) + ["synthetic-not-real", PASSWORD]:
        check(secret not in out and secret not in js, "scan report leaks a secret")
    check(hash_tree() == before, "scan changed an original")
    classes = {k: len(re.findall(rf"\b{k}\b", out)) for k in ("imported", "deferred", "unsupported")}
    return {"classes_mentioned": classes, "report_lines": len(out.splitlines()), "report_head": out.splitlines()[:12]}


def import_users():
    before = hash_tree()
    rc, out = sh("/usr/lib/limeos/limeosctl migrate import-users --env-file /etc/limeos/credentials.env", check_rc=False, log="import-users-1")
    check(rc == 0 and "imported: 2" in out, "import failed", out=out)
    check("scrypt" not in out and "pbkdf2" not in out, "import output leaks a hash")
    legacy = users()
    check(legacy["holly"].startswith("scrypt:") and legacy["bob"].startswith("pbkdf2:"), "stored hashes are not the legacy forms", users={k: v[:12] for k, v in legacy.items()})
    status, headers, view = login("holly", via_https=False)
    check(status == 200, "imported scrypt hash did not log in", status=status, view=view)
    check(login("holly", "wrong-password", via_https=False)[0] == 401, "wrong password was not refused")
    after = users()
    check(after["holly"].startswith("$argon2id$"), "scrypt hash did not upgrade to Argon2id", prefix=after["holly"][:12])
    check(after["bob"] == legacy["bob"], "the other user's hash changed without a login")
    # Idempotent after an upgrade: nothing is overwritten or compared.
    rc, out = sh("/usr/lib/limeos/limeosctl migrate import-users --env-file /etc/limeos/credentials.env", check_rc=False, log="import-users-2")
    check(rc == 0 and "imported: 0" in out and "nothing changed" in out and "skipped existing: 2" in out, "re-run was not a no-op", out=out)
    check(users() == after, "re-run changed a stored hash")
    # A bad entry imports nothing, even for a valid new name in the same file.
    scrypt = legacy["holly"]
    Path("/root/bad.env").write_text(f"PIHEALTH_USERS=carol:{scrypt},dave:scrypt:1$bad$zz\n")
    rc, out = sh("/usr/lib/limeos/limeosctl migrate import-users --env-file /root/bad.env", check_rc=False, log="import-users-bad")
    check(rc != 0, "bad entry was accepted", out=out)
    check(users() == after and "carol" not in users(), "bad file imported something")
    check(hash_tree() == before, "import-users changed the frozen app's files")
    return {"first": out_first(), "upgraded_prefix": after["holly"][:10], "bob_still": after["bob"][:10]}


def out_first():
    return state["logs"]["import-users-1"][-400:]


@case("4 propose-ceiling: deny-by-default, accepted by check-ceiling; operator enable step; ceiling installed (LimeOS not running yet)")
def ceiling():
    docker_ps = Path("/root/docker-ps.jsonl")
    docker_ps.write_text(sh("docker ps --no-trunc --format '{{json .}}'")[1])
    uid = pwd.getpwnam("limeos-core").pw_uid
    proposed = Path("/root/container-proposed.json")
    rc, out = sh(f"/usr/lib/limeos/limeosctl migrate propose-ceiling --docker-ps {docker_ps} --stacks /opt/stacks --core-uid {uid} --out {proposed}", check_rc=False, log="propose-ceiling")
    check(rc == 0, "propose-ceiling failed", out=out)
    value = json.loads(proposed.read_text())
    check(not any(v for k, v in value.items() if k.startswith("allow_")), "proposal enables something", value=value)
    check(sorted(value["managed_containers"]) == sorted(state["ids"][n] for n in RUNNING), "managed containers differ from docker ps")
    check(value["managed_stacks"] == ["mattermost", "media"], "managed_stacks differ", stacks=value["managed_stacks"])
    check(sh(f"/usr/lib/limeos/limeosctl check-ceiling {proposed} {uid} container", check_rc=False)[0] == 0, "check-ceiling refused the proposal")
    state["proposed_ceiling"] = proposed
    # Operator-enable step (scripted): flip exactly the intended flags.
    for key in ("allow_health", "allow_container_read", "allow_container_logs", "allow_restart", "allow_start", "allow_stop", "allow_stack"):
        value[key] = True
    enabled_file = Path("/root/container-enabled.json")
    enabled_file.write_text(json.dumps(value, indent=2) + "\n")
    check(sh(f"/usr/lib/limeos/limeosctl check-ceiling {enabled_file} {uid} container", check_rc=False)[0] == 0, "check-ceiling refused the enabled ceiling")
    shutil_copy(enabled_file, POLICY)
    check(permitted(POLICY) == "root:root 644", "container ceiling is not root:root 0644", mode=permitted(POLICY))
    state["policy"] = value
    state["enabled_ceiling"] = enabled_file
    return {"proposal_listing": out.splitlines()[:12], "managed": len(value["managed_containers"]), "stacks": value["managed_stacks"], "enabled": [k for k, v in value.items() if k.startswith("allow_") and v]}


def shutil_copy(source, destination):
    temporary = f"{destination}.new"
    sh(f"install -m 0644 -o root -g root {source} {temporary} && mv -f {temporary} {destination}")


def preflight_args():
    """Runbook step 2: the state list and the root-only Jellyfin key file (default URL, the jellyfin container)."""
    return ["--state-list", STATE_LIST, "--live-list", LIVE_LIST, "--jellyfin-key-file", KEY_FILE,
            "--host", HOST, "--package", state["deb"], "--expected-version", state["version"]]


def snapshot_ready(name):
    """Fresh preflight; the managed file lists the six running containers."""
    managed = Path("/root/managed.txt")
    managed.write_text("\n".join(RUNNING) + "\n")
    out_dir = f"/var/lib/limeos-cutover/{name}"
    rc, out = run_script("preflight.sh", *preflight_args(), "--out", out_dir, "--managed-file", managed, log=f"preflight-{name}")
    return rc, out, Path(out_dir)


def snapshot_units(snap, units):
    """{unit: (active, enabled)} as the snapshot's units.txt recorded them."""
    recorded = {}
    for line in (Path(snap) / "units.txt").read_text().splitlines():
        fields = line.split()
        if len(fields) == 4 and fields[0] == "managed" and fields[1] in units:
            recorded[fields[1]] = (fields[2], fields[3])
    return recorded


def deb_args(version=None, host=HOST, port=PORT, extra=()):
    return ["--package", state["deb"], "--expected-version", version or state["version"], "--host", host, "--port", str(port), *extra]


def snap_summary(directory):
    return {n: (directory / n).read_text()[:600] for n in ("units.txt", "docker-ps.txt", "proxy.txt", "findings.txt", "meta.env", "pagesize.txt", "packages.txt")}


def expect_frozen_serving(label, snap=None, rust_running=False):
    """Frozen app serves plain HTTP on 8002 as holly, caddy inactive, the Rust units stopped and disabled.

    The frozen app owns /var/lib/limeos (StateDirectory=limeos), as on wybie: configure does not start the
    Rust units while it does, and return.sh leaves them stopped and disabled whatever the snapshot recorded
    (since 26 Nov). The directory is holly:pihealth 0750 again and LimeOS's trees in it are still LimeOS's."""
    status, body = plain_get(8002)
    check(status == 200 and "pi-health-imitation" in body, f"{label}: frozen app not serving over plain HTTP", status=status, body=body[:200])
    check(active("caddy") == "inactive" and enabled("caddy") == "disabled", f"{label}: caddy not inactive/disabled")
    check(not Path("/etc/caddy/conf.d/limeos-cutover.caddy").exists(), f"{label}: Caddy site left behind")
    expect_state_dir_frozen(label)
    for u in RUST_UNITS:
        if rust_running:
            check(active(u) == "active" and enabled(u) == "enabled", f"{label}: {u} not active+enabled", state=(active(u), enabled(u)))
        else:
            check(active(u) in ("inactive", "failed") and enabled(u) == "disabled", f"{label}: {u} not stopped+disabled", state=(active(u), enabled(u)))
    check(frozen_world() == {u: {"active": "active", "enabled": "enabled"} for u in FROZEN_UNITS}, f"{label}: frozen units not restored", world=frozen_world())
    ports = listeners()
    reader = (active("limeos-storage-reader"), enabled("limeos-storage-reader"))
    # Only activate.sh starts it, only return.sh/rollback stops it, nothing enables it; no caller of this helper is activated.
    check(reader[0] != "active" and reader[1] != "enabled", f"{label}: stack reader running or enabled", reader=reader)
    check(("127.0.0.1:8003" in ports) == rust_running, f"{label}: core listening state", ports=ports)
    owner = sh("ps -o user=,comm= -p $(systemctl show pi-health -p MainPID --value)")[1].split()
    check(owner[0] == "holly", f"{label}: frozen app owner", owner=owner)
    check("caddy" not in ports, f"{label}: caddy still listening", ports=ports)


def expect_world_equal(label, before_containers, before_bystanders, strict=True):
    check(containers() == before_containers, f"{label}: container IDs/StartedAt/pids differ", before=before_containers, after=containers())
    check(bystanders() == before_bystanders, f"{label}: a bystander unit changed", before=before_bystanders, after=bystanders())
    check(hash_tree() == state["frozen_hashes"], f"{label}: a frozen-app file hash changed", changed=[p for p, h in hash_tree().items() if state["frozen_hashes"].get(p) != h])


PACKAGE_PATHS = ("/etc/limeos/core.json", "/etc/limeos/system-policy/", "/var/lib/limeos/core/", "/var/lib/limeos/executors/", "/var/lib/limeos/assistant/")


@case("4a runbook step 1: state list built by the runbook's own commands from the scan report; root-only Jellyfin key file")
def state_list():
    """Runs the runbook's sh block exactly as written. If it does not produce a usable list, the case fails
    and later cases fall back to the same block with limeosctl's full path, so their results stay readable."""
    block = runbook_block("Build the state list")
    STATE_LIST.unlink(missing_ok=True)
    LIVE_LIST.unlink(missing_ok=True)
    rc, out = sh(block, check_rc=False, log="runbook-state-list")
    produced = STATE_LIST.read_text() if STATE_LIST.exists() else ""
    paths = [p for p in produced.splitlines() if p.strip()]
    problems = []
    if rc != 0 or not paths:
        problems.append(f"the runbook's commands exited {rc} and listed {len(paths)} path(s): {out[-600:]}")
        fallback = re.sub(r"(?<![/\w])limeosctl ", "/usr/lib/limeos/limeosctl ", block)
        rc2, out2 = sh(fallback, check_rc=False, log="runbook-state-list-fullpath")
        paths = [p for p in STATE_LIST.read_text().splitlines() if p.strip()] if STATE_LIST.exists() else []
        state["notes"].append(f"state list: the runbook block failed; later cases use it with /usr/lib/limeos/limeosctl (exit {rc2}, {len(paths)} paths)")
    # The runbook says to drop what is not the frozen app's; nothing here edits the list by hand.
    package_owned = [p for p in paths if p.startswith(PACKAGE_PATHS) or p == "/etc/limeos/core.json"]
    if package_owned:
        problems.append(f"the list names {len(package_owned)} file(s) of the LimeOS package itself, which change while the cutover runs: {package_owned}")
    for wanted in ("/etc/limeos/credentials.env", "/home/holly/pi-health/config/backup_config.json", "/home/holly/pi-health/config/media_paths.json", "/var/lib/limeos/metrics.sqlite3"):
        if wanted not in paths:
            problems.append(f"the list misses the frozen app's {wanted}")
    # Reviewable: no virtualenv file, nothing the running limeops services write.
    if any("/venv/" in p for p in paths) or any(p.startswith(LIVE_PATHS) or p in LIVE_PATHS for p in paths):
        problems.append(f"the state list holds virtualenv or live paths: {[p for p in paths if '/venv/' in p or p.startswith(LIVE_PATHS)][:8]}")
    live = [l for l in LIVE_LIST.read_text().splitlines() if l and not l.startswith("#")] if LIVE_LIST.exists() else []
    if sorted(live) != sorted(LIVE_PATHS):
        problems.append(f"the live list differs from the runbook's entries: {live}")
    if any(not p.startswith("/") for p in paths) or any(p.startswith("/opt/stacks") for p in paths):
        problems.append("the list holds a relative path or a stacks file")
    # Jellyfin API key, root-only, written with an editor (here: appended by Python, never on a command line).
    rc_key, out_key = sh(runbook_key_command(), check_rc=False, log="runbook-key-file")
    with KEY_FILE.open("a") as handle:
        handle.write(JELLYFIN_KEY + "\n")
    if rc_key != 0 or permitted(KEY_FILE) != "root:root 600":
        problems.append(f"the runbook's key-file command did not leave a root-only file ({rc_key}, {permitted(KEY_FILE)}): {out_key}")
    state["state_list"] = paths
    check(not problems, "; ".join(problems))
    return {"paths": paths, "live": LIVE_LIST.read_text() if LIVE_LIST.exists() else None, "helper_output": out.strip().splitlines()[-12:], "key_file": permitted(KEY_FILE)}


def runbook_key_command():
    text = RUNBOOK.read_text()
    found = re.search(r"`(install -o root -g root -m 0600 /dev/null /root/jellyfin\.key)`", text)
    check(found, "runbook names no key-file install command")
    return found.group(1)


@case("4b propose-catalog: preflight refuses without a catalog (exact command); proposal from Compose labels; --install root 0644; never overwrites")
def catalog():
    ctl = "/usr/lib/limeos/limeosctl"
    check(not CATALOG.exists(), "a catalog exists before propose-catalog")
    managed = Path("/root/managed.txt")
    managed.write_text("\n".join(RUNNING) + "\n")
    rc, out = run_script("preflight.sh", *preflight_args(), "--out", "/var/lib/limeos-cutover/no-catalog", "--managed-file", managed, log="preflight-no-catalog")
    check(rc != 0 and "management-catalog.json is missing" in out and f"{ctl} migrate propose-catalog --out" in out, "preflight did not refuse a missing catalog with the command", out=out[-800:])
    proposed = Path("/root/management-catalog.proposed.json")
    rc, out = sh(f"{ctl} migrate propose-catalog --out {proposed}", check_rc=False, log="propose-catalog")
    check(rc == 0, "propose-catalog failed", out=out)
    check(not CATALOG.exists(), "propose-catalog installed the catalog without --install")
    value = json.loads(proposed.read_text())
    engine_id = sh("docker info --format '{{.ID}}'")[1].strip()
    expected = {
        "version": 1,
        "engine_id": engine_id,
        "stacks": [{
            "stack": "mattermost",
            "project": "mattermost",
            "working_directory": "/opt/stacks/mattermost",
            "operator": {"name": "compose.yaml", "sha256": sha(Path("/opt/stacks/mattermost/compose.yaml"))},
            "managed": {"state": "absent"},
            "services": sorted(MATTERMOST),
        }, {
            "stack": "media",
            "project": "media",
            "working_directory": "/opt/stacks/media",
            "operator": {"name": "compose.yaml", "sha256": sha(Path("/opt/stacks/media/compose.yaml"))},
            "managed": {"state": "absent"},
            "services": ["airsonic-advanced", "jellyfin", "radarr", "sonarr", "transmission"],
        }],
    }
    check(value == expected, "proposed catalog differs from the Compose projects", value=value)
    check("stack media (project media)" in out and "stack mattermost (project mattermost)" in out and "refused project docker" in out
          and "container(s) without a Compose project" in out, "proposal listing incomplete", out=out)
    again = sh(f"{ctl} migrate propose-catalog --out {proposed}", check_rc=False)
    check(again[0] != 0 and "already exists" in again[1], "propose-catalog overwrote its output", out=again[1])
    # The operator installs it with --install (a second proposal file, same content).
    second = Path("/root/management-catalog.second.json")
    rc, out2 = sh(f"{ctl} migrate propose-catalog --out {second} --install", check_rc=False, log="propose-catalog-install")
    check(rc == 0 and CATALOG.exists(), "--install did not install the catalog", out=out2)
    st = CATALOG.stat()
    check(st.st_uid == 0 and st.st_gid == 0 and oct(st.st_mode & 0o7777) == "0o644", "installed catalog is not root:root 0644", mode=oct(st.st_mode))
    check(json.loads(CATALOG.read_text()) == expected, "installed catalog differs from the proposal")
    third = sh(f"{ctl} migrate propose-catalog --out /root/management-catalog.third.json --install", check_rc=False)
    check(third[0] != 0 and "already exists" in third[1] and json.loads(CATALOG.read_text()) == expected, "--install replaced an installed catalog", out=third[1])
    check(active("limeos-storage-reader") != "active", "stack reader running before the cutover")
    shutil.rmtree("/var/lib/limeos-cutover/no-catalog", ignore_errors=True)
    return {"listing": out.splitlines(), "catalog": expected}


@case("5 preflight refusals leave everything unchanged (stopped managed container; active playback)")
def preflight_refusals():
    before_c = containers()
    before_b = bystanders()
    managed = Path("/root/managed-bad.txt")
    managed.write_text("\n".join([*RUNNING, "airsonic-advanced"]) + "\n")
    rc, out = run_script("preflight.sh", *preflight_args(), "--out", "/var/lib/limeos-cutover/refused-stopped", "--managed-file", managed, log="preflight-refused-stopped")
    check(rc != 0 and "managed container not running: airsonic-advanced" in out, "stopped managed container did not refuse", out=out)
    # No key file and no --no-playback-check: refused, naming both ways out.
    managed.write_text("\n".join(RUNNING) + "\n")
    rc0, out0 = run_script("preflight.sh", "--state-list", STATE_LIST, "--out", "/var/lib/limeos-cutover/refused-nokey", "--managed-file", managed, log="preflight-refused-nokey")
    check(rc0 != 0 and "no Jellyfin API key: pass --jellyfin-key-file FILE (root-only) or --no-playback-check" in out0, "preflight without a key file did not refuse", out=out0)
    # Active playback: a tiny local server answers /Sessions with a NowPlayingItem.
    seen = []

    class Sessions(BaseHTTPRequestHandler):
        def do_GET(self):
            seen.append(self.headers.get("Authorization"))
            body = b'[{"NowPlayingItem": {"Name": "x"}}]'
            self.send_response(200)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def log_message(self, *args):
            pass

    server = HTTPServer(("127.0.0.1", 18096), Sessions)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    try:
        rc2, out2 = run_script("preflight.sh", *preflight_args(), "--out", "/var/lib/limeos-cutover/refused-playback", "--managed-file", managed, "--jellyfin-url", "http://127.0.0.1:18096/Sessions", log="preflight-refused-playback")
    finally:
        server.shutdown()
    check(rc2 != 0 and "Jellyfin has active playback" in out2, "active playback did not refuse", out=out2)
    check(seen == [f'MediaBrowser Token="{JELLYFIN_KEY}"'], "the key did not reach Jellyfin as the MediaBrowser token", seen=seen)
    snap_files = sh("grep -rl -- " + JELLYFIN_KEY + " /var/lib/limeos-cutover", check_rc=False)[1].strip()
    check(not snap_files, "the Jellyfin key was written into a snapshot", files=snap_files)
    # A refused snapshot cannot be activated.
    rc3, out3 = run_script("activate.sh", "/var/lib/limeos-cutover/refused-playback", "--yes-i-have-operator-approval", *deb_args(), log="activate-refused-snapshot")
    check(rc3 != 0 and "refused preflight" in out3, "activate accepted a refused snapshot", out=out3)
    expect_world_equal("after refusals", before_c, before_b)
    expect_frozen_serving("after refusals")
    return {"stopped": out.strip().splitlines()[-2:], "no_key": out0.strip().splitlines()[-2:], "playback": out2.strip().splitlines()[-2:], "activate_on_refused": out3.strip().splitlines()[-1:]}


@case("6 cycle 1: preflight, dry-run (no change), activate; invariants; HTTPS login, container list/status/logs")
def cycle1_activate():
    before_c = containers()
    before_b = bystanders()
    state["c1_containers"] = before_c
    rc, out, snap = snapshot_ready("snap1")
    check(rc == 0 and "preflight passed" in out, "preflight did not pass", out=out)
    # Kernel 6.1 with the unified hierarchy: return.sh --force can use cgroup.kill, so no warning; nothing else either
    # (LimeOS is stopped, Caddy is stopped, the activation inputs were checked).
    check("cgroup.kill is not available" not in out and "WARN" not in out, "preflight warned", out=out)
    check((snap / "state-claimants.txt").read_text().split() == CLAIMANTS, "preflight did not record who owns /var/lib/limeos", claimants=(snap / "state-claimants.txt").read_text())
    check("docker-compose-start" not in (snap / "units.txt").read_text(), "a unit the cutover does not manage was recorded")
    check((snap / "state-paths.txt").read_text().split("\n")[:-1] == state["state_list"], "snapshot's state-paths.txt is not the state list")
    state["snap1"] = snap
    snap_text = snap_summary(snap)
    check("limeos" in snap_text["packages.txt"], "package not recorded")
    # Dry run: prints, changes nothing.
    unit_before = {u: (active(u), enabled(u)) for u in [*FROZEN_UNITS, *RUST_UNITS, "caddy"]}
    rc, out = run_script("activate.sh", snap, "--dry-run", *deb_args(), log="activate-dry-run")
    check(rc == 0 and "[dry-run]" in out, "dry-run failed", out=out[-1500:])
    check({u: (active(u), enabled(u)) for u in [*FROZEN_UNITS, *RUST_UNITS, "caddy"]} == unit_before, "dry-run changed unit state")
    expect_world_equal("after dry-run", before_c, before_b)
    check(plain_get(8002)[0] == 200, "frozen app stopped by dry-run")
    # A holly-owned subtree inside one of LimeOS's trees (what a recursive chown leaves) holding a symlink to
    # a root-owned file outside: the takeover must give the subtree back without following the link.
    outside = Path("/root/outside-the-tree")
    outside.write_text("must stay root's\n")
    planted = Path("/var/lib/limeos/assistant/planted")
    planted.mkdir()
    (planted / "escape").symlink_to(outside)
    (planted / "dirlink").symlink_to("/etc")
    sh(f"chown -h -R holly:pihealth {planted}")
    # Let the collector write once more so the SQLite file really changes between preflight and the stop.
    time.sleep(22)
    started = time.monotonic()
    rc, out = run_script("activate.sh", snap, "--yes-i-have-operator-approval", *deb_args(), log="activate-1")
    elapsed = round(time.monotonic() - started, 1)
    check(rc == 0 and "activate complete" in out, "activate failed", out=out[-3000:])
    # The state directory is LimeOS's, the old units are guarded, LimeOS's trees are its own.
    check(owner_mode("/var/lib/limeos") == (0, 0, 0o755), "/var/lib/limeos is not root:root 0755 after activation", now=permitted("/var/lib/limeos"))
    check(all(Path(f"/etc/systemd/system/{u}.d/{GUARD}").exists() for u in CLAIMANTS), "the cutover guards are missing")
    for tree, account in PACKAGE_TREES.items():
        check(not foreign_entries(tree, account), f"{tree} has entries not owned by {account} after activation", entries=foreign_entries(tree, account)[:10])
    check(owner_mode(outside)[:2] == (0, 0) and owner_mode("/etc")[:2] == (0, 0), "the takeover followed a planted symlink out of the tree", outside=owner_mode(outside), etc=owner_mode("/etc"))
    check(os.lstat(planted / "escape").st_uid == pwd.getpwnam("limeos-assistant").pw_uid, "the planted symlink itself was not given back")
    # The limeops writer, an account in group pihealth, still writes through the root:root 0755 directory.
    beats = lambda: sqlite3.connect(f"file:{SUPERVISION_DB}?mode=ro", uri=True).execute("select count(*) from beats").fetchone()[0]  # noqa: E731
    before_beats = beats()
    fx.eventually(lambda: beats() > before_beats, timeout=20)
    check(active("limeops-worker.service") == "active", "the limeops writer stopped after the takeover")
    # Detached by default, inside the snapshot's transient scope, which is gone once the run ends.
    scope = (snap / "activation-scope").read_text().strip()
    check(re.fullmatch(r"limeos-cutover-activate-[0-9a-f]{32}\.scope", scope) and f"running detached in scope {scope}" in out, "activation did not run detached in its scope", out=out[:800])
    check((snap / "activated").exists() and active(scope) == "inactive", "snapshot not marked activated or scope still active", scope=active(scope))
    # Activation itself changed no container, no bystander, no frozen-app file.
    expect_world_equal("after activation", before_c, before_b)
    check(frozen_world() == {u: {"active": "inactive", "enabled": "disabled"} for u in FROZEN_UNITS}, "frozen units not stopped+disabled", world=frozen_world())
    check(all(active(u) == "active" and enabled(u) == "enabled" for u in RUST_UNITS), "Rust services not active+enabled")
    check(active("caddy") == "active" and enabled("caddy") == "enabled", "caddy not active+enabled")
    check(active("limeos-storage-reader") == "active" and enabled("limeos-storage-reader") != "enabled", "activate.sh did not leave the stack reader started and not enabled", state=(active("limeos-storage-reader"), enabled("limeos-storage-reader")))
    check("stack reader still ready after settling" in out, "activate.sh did not verify the stack reader", out=out[-800:])
    ports = listeners()
    check(re.search(r"\*:8002\s.*caddy", ports) or re.search(r"8002\s.*caddy", ports), "8002 is not Caddy's", ports=ports)
    check("127.0.0.1:8003" in ports and "limeos-core" in ports, "core not on loopback 8003", ports=ports)
    check("pi-health" not in sh("pgrep -af app.py", check_rc=False)[1], "frozen app still running")
    check(plain_get(8002)[0] != 200, "plain HTTP still answered on 8002")
    # Frozen SQLite is quiescent while the writers are stopped.
    a = sqlite_facts()
    time.sleep(25)
    b = sqlite_facts()
    check(a == b and a["integrity"] == "ok", "frozen SQLite changed while writers are stopped", a=a, b=b)
    state["poststop_baseline_exists"] = (snap / "state.poststop.sha256").exists()
    # 26 Nov: a start of pi-health (by the rollback then; by hand, or by its timer, at any time) chowned
    # /var/lib/limeos and LimeOS's trees to holly. The guard makes a hand start fail and change nothing.
    guard_rc, guard_out = sh("systemctl start pi-health.service", check_rc=False, log="start-guarded-pi-health")
    collector_rc, _ = sh("systemctl start limeos-metrics-collector.service", check_rc=False, log="start-guarded-collector")
    check(guard_rc != 0 and "ssertion" in guard_out and collector_rc != 0, "a start of the guarded frozen units did not fail", out=guard_out)
    check(active("pi-health.service") != "active" and owner_mode("/var/lib/limeos") == (0, 0, 0o755), "the guarded start changed something", now=permitted("/var/lib/limeos"))
    check(sh("/usr/lib/limeos/limeosctl --socket /run/limeos-core/core.sock status", check_rc=False)[0] == 0, "core does not answer after the guarded starts")
    check(active("docker-compose-start.service") == "failed", "the pre-existing failed unit changed state")
    return {"guarded_start": guard_out.strip().splitlines()[-2:], "activate_seconds": elapsed, "activate_output_tail": out.strip().splitlines()[-12:], "poststop_baseline": state["poststop_baseline_exists"], "listeners": ports}


@case("6a import-users right after activation (runbook step 1g): Werkzeug scrypt and PBKDF2 hashes imported; login works; hash upgrades; idempotent; bad entry imports nothing; a deny-by-default ceiling refuses a plan")
def import_users_after_activation():
    detail = import_users()
    # Nothing acts before the flags are set: the ceiling as proposed refuses a restart plan.
    shutil_copy(state["proposed_ceiling"], POLICY)
    sh("systemctl reset-failed limeos-containerd; systemctl restart limeos-containerd")
    fx.eventually(lambda: active("limeos-containerd") == "active", timeout=30)
    time.sleep(3)  # allow_health is false in the proposal: the executor's health probe is refused by design
    status, headers, view = login("holly", via_https=False)
    check(status == 200, "login on the core failed", status=status)
    cookie, csrf = headers["set-cookie"].split(";")[0], view["csrf_token"]
    status, _, refused = fx.http("/api/v1/container/plans", "POST", {"resource": "container:" + state["ids"]["radarr"], "operation": "restart"}, cookie, csrf, origin=SITE)
    check(status != 201, "a deny-by-default ceiling allowed a restart plan", status=status, body=refused)
    shutil_copy(state["enabled_ceiling"], POLICY)
    sh("systemctl reset-failed limeos-containerd; systemctl restart limeos-containerd")
    fx.wait_container_ready(timeout=45)
    return {**detail, "denied_plan_status": status}


@case("7 cycle 1 (activated): HTTPS login with the internal CA; list/status/logs; stack view; host metrics")
def cycle1_https():
    check(CA.exists(), "Caddy internal CA not created")
    # Certificate is issued by Caddy's local CA for the host name and verified by Python's TLS stack.
    status, headers, view = login("bob")
    check(status == 200, "PBKDF2 imported hash did not log in through HTTPS", status=status, view=view)
    check(users()["bob"].startswith("$argon2id$"), "PBKDF2 hash did not upgrade after HTTPS login")
    status2, headers2, view2 = login("holly")
    check(status2 == 200, "Argon2id (already upgraded) login failed through HTTPS", status=status2)
    check(login("holly", "nope")[0] == 401, "wrong password accepted through HTTPS")
    cookie = headers2["set-cookie"].split(";")[0]
    csrf = view2["csrf_token"]
    state["session"] = {"cookie": cookie, "csrf": csrf}
    check("secure" in headers2["set-cookie"].lower() or "Secure" in headers2["set-cookie"], "session cookie is not Secure", cookie=headers2["set-cookie"][:80])
    headers_seen = {k.lower(): v for k, v in https("/", cookie=cookie)[1].items()}
    check(headers_seen.get("x-content-type-options") == "nosniff" and "content-security-policy" in headers_seen, "proxy headers missing", headers=list(headers_seen))
    # Plain HTTP login attempt to the public port is not served.
    check(plain_get(8002)[0] != 200, "plain HTTP still served")
    status, _, overview = https("/api/v1/overview", cookie=cookie)
    check(status == 200, "overview failed", status=status)
    resources = overview["resources"]
    kinds = sorted({r["kind"] for r in resources})
    listed = {r["id"]: r.get("status") for r in resources if r["kind"] == "container"}
    check(all("container:" + state["ids"][n] in listed for n in RUNNING), "a running container is missing from the list", listed=listed)
    check(all(listed["container:" + state["ids"][n]] == "running" for n in RUNNING), "a running container is not reported running", listed=listed)
    status, _, logs = https(f"/api/v1/containers/{state['ids']['jellyfin']}/logs?tail=50", cookie=cookie)
    check(status == 200 and "jellyfin ready" in logs["text"], "container logs not served", status=status, logs=str(logs)[:300])
    unmanaged = https(f"/api/v1/containers/{state['ids']['airsonic-advanced']}/logs?tail=50", cookie=cookie)[0]
    check(unmanaged == 403, "unmanaged container logs not refused", status=unmanaged)
    # Stack status is available right after activate.sh with no manual step (reader started, catalog proposed).
    def stacks_fresh():
        status, _, value = https("/api/v1/deployment/stacks", cookie=cookie)
        return status == 200 and value["catalog"]["state"] == "fresh" and value["engine"]["state"] == "fresh" and any(x["project"] == "media" and not x["issues"] for x in value["stacks"])

    waited = time.monotonic()
    fx.eventually(stacks_fresh, timeout=60)
    stack_wait = round(time.monotonic() - waited, 1)
    status, _, stacks = https("/api/v1/deployment/stacks", cookie=cookie)
    check(stacks["catalog"]["state"] != "unavailable" and stacks["engine"]["state"] != "unavailable", "stack status is unavailable after activate.sh", stacks=str(stacks)[:600])
    host = https("/api/v1/resources", cookie=cookie)
    return {"overview_kinds": kinds, "overview_keys": sorted(overview), "containers_listed": len(listed), "log_first_line": logs["text"].splitlines()[:2], "unmanaged_logs": unmanaged, "stacks_status": status, "stack_status_wait_seconds": stack_wait, "stacks": str(stacks)[:900], "resources_status": host[0], "resources": str(host[2])[:600]}


@case("8 cycle 1: return.sh restores the frozen app; containers/hashes/bystanders identical to the preflight snapshot; idempotent")
def cycle1_return():
    before_c = state["c1_containers"]
    rc, out = run_script("return.sh", state["snap1"], log="return-1")
    check(rc == 0 and "return verified" in out, "return.sh failed", problems=problems(out), out=out[-1500:])
    # The limeops writer kept writing through the whole cycle: reported, not failed on.
    check("live state changed since preflight (not a failure" in out and "supervision.sqlite3" in out, "the live writer's changes were not reported", out=out[-2500:])
    check("leaving the LimeOS services stopped and disabled: /var/lib/limeos belongs to pi-health.service limeos-metrics-collector.service again" in out, "return.sh did not say why LimeOS is left stopped", out=out[-2500:])
    check(Path(fx.CORE_DB).exists() and users(), "core's database or its users did not survive the return")
    expect_frozen_serving("after return", state["snap1"])
    expect_world_equal("after return", before_c, state["baseline_bystanders"])
    s1 = sqlite_facts()
    rc2, out2 = run_script("return.sh", state["snap1"], log="return-1-again")
    check(rc2 == 0 and "return verified" in out2, "return.sh is not idempotent", problems=problems(out2), out=out2[-1500:])
    expect_frozen_serving("after second return", state["snap1"])
    expect_world_equal("after second return", before_c, state["baseline_bystanders"])
    return {"first": out.strip().splitlines()[-6:], "second": out2.strip().splitlines()[-3:], "sqlite": [s1, sqlite_facts()]}


# --- cycle 2: activate again, exercise approved operations ------------------------------------------------------
def plan_cycle(path_prefix, body, key, cookie, csrf, retry=False):
    deadline = time.monotonic() + (90 if retry else 0)
    while True:
        status, _, proposal = https(f"{path_prefix}/plans", "POST", body, cookie, csrf)
        if status == 201 or status not in (409, 503) or time.monotonic() >= deadline:
            break
        time.sleep(2)
    check(status == 201, "plan refused", status=status, body=proposal)
    status, _, approval = https(f"{path_prefix}/plans/{proposal['plan']['id']}/approval", "POST", {"digest": proposal["digest"]}, cookie, csrf)
    check(status == 200, "approval refused", status=status, body=approval)
    status, _, job = https(f"{path_prefix}/jobs", "POST", {"proposal": proposal, "approval": approval["token"], "idempotency_key": key}, cookie, csrf)
    check(status == 202, "queue refused", status=status, body=job)
    final = {}

    def done():
        final["state"] = fx.state(job["id"])
        return final["state"] in ("succeeded", "failed", "canceled", "precondition_changed", "needs_intervention", "outcome_unknown")

    fx.eventually(done, timeout=120)
    return job["id"], final["state"]


def docker_events(since, until):
    out = sh(f"docker events --since {since} --until {until} --filter type=container --format '{{{{json .}}}}'", timeout=60)[1]
    events = []
    for line in out.splitlines():
        try:
            e = json.loads(line)
        except json.JSONDecodeError:
            continue
        events.append((e.get("Actor", {}).get("Attributes", {}).get("name"), (e.get("Action") or e.get("status") or "").split(":")[0]))
    return events


@case("9 cycle 2: preflight + activate again (package already installed, users already imported)")
def cycle2_activate():
    before_c = containers()
    state["c2_containers"] = before_c
    rc, out, snap = snapshot_ready("snap2")
    check(rc == 0, "preflight did not pass", out=out)
    state["snap2"] = snap
    rc, out = run_script("activate.sh", snap, "--yes-i-have-operator-approval", *deb_args(), log="activate-2")
    check(rc == 0 and "activate complete" in out, "second activate failed", out=out[-3000:])
    expect_world_equal("after activation 2", before_c, state["baseline_bystanders"])
    status, headers, view = login("holly")
    check(status == 200, "login after re-activation failed", status=status)
    state["session"] = {"cookie": headers["set-cookie"].split(";")[0], "csrf": view["csrf_token"]}
    return {"tail": out.strip().splitlines()[-4:]}


@case("10 approved container restart through the HTTPS API: exactly one container restarted, nothing else")
def container_restart():
    s = state["session"]
    before = containers()
    t0 = int(time.time()) - 1
    job, final = plan_cycle("/api/v1/container", {"resource": "container:" + state["ids"]["radarr"], "operation": "restart"}, "cutover-radarr-restart", s["cookie"], s["csrf"], retry=True)
    time.sleep(2)
    after = containers()
    events = docker_events(t0, int(time.time()) + 1)
    check(final == "succeeded", "restart job did not succeed", final=final)
    check(after["radarr"]["started"] != before["radarr"]["started"] and after["radarr"]["running"] and after["radarr"]["id"] == before["radarr"]["id"], "radarr was not restarted in place", before=before["radarr"], after=after["radarr"])
    others = {n: v for n, v in before.items() if n != "radarr"}
    check({n: after[n] for n in others} == others, "another container changed", before=others)
    touched = sorted({n for n, a in events if a in ("restart", "stop", "start", "kill", "die")})
    check(touched == ["radarr"], "Engine events touched other containers", events=events)
    return {"job": job, "events": events}


@case("11 whole-stack restart (media) through the HTTPS API: only the four running members, ID order, stopped member untouched")
def stack_restart():
    s = state["session"]
    # activate.sh started the reader; nothing here starts it by hand any more.
    reader_before = active("limeos-storage-reader")
    check(reader_before == "active", "stack reader not started by activate.sh", state=reader_before)

    def inventory_ok():
        status, _, value = https("/api/v1/deployment/stacks", cookie=s["cookie"])
        if status != 200 or value["catalog"]["state"] != "fresh" or value["engine"]["state"] != "fresh":
            return False
        row = next((x for x in value["stacks"] if x["project"] == "media"), None)
        return bool(row) and not row["issues"]

    try:
        fx.eventually(inventory_ok, timeout=120)
    except AssertionError:
        raise AssertionError("stack inventory never became fresh: " + str(https("/api/v1/deployment/stacks", cookie=s["cookie"])[2])[:1500]) from None
    before = containers()
    t0 = int(time.time()) - 1
    job, final = plan_cycle("/api/v1/stack", {"project": "media", "operation": "restart"}, "cutover-media-restart", s["cookie"], s["csrf"], retry=True)
    time.sleep(2)
    after = containers()
    events = docker_events(t0, int(time.time()) + 1)
    check(final == "succeeded", "stack restart did not succeed", final=final)
    for name in MEDIA_RUNNING:
        check(after[name]["started"] != before[name]["started"] and after[name]["running"] and after[name]["id"] == before[name]["id"], f"{name} not restarted in place", before=before[name], after=after[name])
    check(after["airsonic-advanced"] == before["airsonic-advanced"] and after["watchtower"] == before["watchtower"] and after["vpn"] == before["vpn"], "a non-member changed")
    touched = sorted({n for n, a in events if a == "restart"})
    check(touched == sorted(MEDIA_RUNNING), "restart events differ from the four running members", events=events)
    return {"job": job, "reader_before": reader_before, "events": events}


@case("12 return.sh after approved restarts reports the changed StartedAt (it detects, still restores the frozen app)")
def cycle2_return():
    rc, out = run_script("return.sh", state["snap2"], log="return-2")
    check(rc != 0, "return.sh did not notice the restarted containers", out=out[-1500:])
    check("StartedAt differs" in out, "return.sh did not name the StartedAt difference", problems=problems(out), out=out[-1500:])
    unexpected = [line for line in problems(out) if line.startswith(("FAIL", "ERROR")) and "StartedAt differs" not in line and "return incomplete" not in line]
    check(not unexpected, "return.sh reported more than the restarted containers", problems=unexpected)
    expect_frozen_serving("after return 2", state["snap2"])
    return {"tail": out.strip().splitlines()[-5:]}


# --- cycle 3: failed activations --------------------------------------------------------------------------------
def failed_activation(label, snap_name, extra, expect_text, version=None):
    before_c = containers()
    before_b = bystanders()
    rc, out, snap = snapshot_ready(snap_name)
    check(rc == 0, "preflight did not pass", out=out)
    rc, out = run_script("activate.sh", snap, "--yes-i-have-operator-approval", *deb_args(version=version, extra=extra), log=f"activate-{label}", timeout=600)
    check(rc == 1, "activate did not fail", rc=rc, out=out[-2500:])
    check(expect_text in out and "ROLLBACK: running return.sh" in out and "return verified" in out, "no automatic verified rollback", problems=problems(out), out=out[-1500:])
    check("stopped unexpectedly" not in out, "a rolled-back activation was reported as an unexpected stop", problems=problems(out))
    expect_frozen_serving(f"after {label}", snap)
    # The frozen app is serving again; containers and files are exactly as before.
    check({n: v for n, v in containers().items()} == before_c, "containers changed", before=before_c)
    check(bystanders() == before_b, "bystanders changed")
    check(hash_tree() == state["frozen_hashes"], "frozen files changed")
    rc2, out2 = run_script("return.sh", snap, log=f"return-{label}-again")
    check(rc2 == 0 and "return verified" in out2, "return.sh after rollback is not idempotent", problems=problems(out2), out=out2[-1500:])
    expect_frozen_serving(f"after {label} + return", snap)
    return {"activate_tail": out.strip().splitlines()[-14:], "return_again_tail": out2.strip().splitlines()[-3:]}


@case("13 failed activation (proxy cannot reach the core: health check fails): automatic rollback, frozen app serving, return.sh idempotent")
def failed_health():
    template = Path("/root/bad-template.caddy")
    template.write_text((SRC / "packaging/Caddyfile.cutover.example").read_text().replace("127.0.0.1:8003", "127.0.0.1:8999"))
    return failed_activation("failed-health", "snap3", ["--template", str(template)], "health check failed: core health via the proxy")


@case("14 wrong --expected-version: refused from the .deb's own Version field before any change; frozen app never stopped")
def failed_version():
    """Since 19 Nov activate.sh reads Package and Version from the named .deb and refuses before the first
    change, so a wrong --expected-version no longer reaches the package step (which used to roll back)."""
    before_c = containers()
    before_b = bystanders()
    rc, out, snap = snapshot_ready("snap4")
    check(rc == 0, "preflight did not pass", out=out)
    pid_before = unit("pi-health.service")["MainPID"]
    rc, out = run_script("activate.sh", snap, "--yes-i-have-operator-approval", *deb_args(version="9.9.9"), log="activate-failed-version", timeout=600)
    check(rc == 1 and f"is version {state['version']}, not 9.9.9" in out, "wrong version not refused from the package fields", rc=rc, out=out[-2500:])
    check("ROLLBACK" not in out and "+ systemctl" not in out, "a refused activation changed something", out=out[-2500:])
    check(unit("pi-health.service")["MainPID"] == pid_before, "the frozen app was restarted")
    expect_frozen_serving("after a version refusal", snap)
    expect_world_equal("after a version refusal", before_c, before_b)
    return {"activate_tail": out.strip().splitlines()[-3:]}


@case("15 negative: activate refuses without approval, with a stale snapshot, with the wrong origin; changes nothing")
def activation_refusals():
    before_c = containers()
    rc, out, snap = snapshot_ready("snap5")
    check(rc == 0, "preflight", out=out)
    rc1, out1 = run_script("activate.sh", snap, *deb_args(), log="activate-no-approval")
    check(rc1 != 0 and "operator-approval" in out1, "activated without approval", out=out1)
    rc2, out2 = run_script("activate.sh", snap, "--yes-i-have-operator-approval", *deb_args(host="other.lan"), log="activate-wrong-origin")
    check(rc2 != 0 and "origin must be" in out2, "wrong origin not refused", out=out2)
    check("stopped unexpectedly" not in out2, "a refused detached activation was reported as an unexpected stop", problems=problems(out2))
    Path("/root/stale-snap").mkdir(exist_ok=True)
    sh(f"rm -rf /root/stale-snap/* && cp -a {snap}/. /root/stale-snap/ && sed -i 's/^epoch=.*/epoch=1/' /root/stale-snap/meta.env")
    rc3, out3 = run_script("activate.sh", "/root/stale-snap", "--yes-i-have-operator-approval", *deb_args(), log="activate-stale")
    check(rc3 != 0 and "stale" in out3, "stale snapshot accepted", out=out3)
    expect_frozen_serving("after refusals 2", snap)
    check(containers() == before_c, "containers changed")
    return {"no_approval": out1.strip().splitlines()[-1], "wrong_origin": out2.strip().splitlines()[-1], "stale": out3.strip().splitlines()[-1]}


# --- since 6 Nov: measure.sh, return.lock, forced return, unit hardening, maintainer scripts --------------------
def activated(name):
    """Runbook steps 2 and 4: a fresh preflight, then a real (detached) activation that must succeed."""
    rc, out, snap = snapshot_ready(name)
    check(rc == 0 and "preflight passed" in out, "preflight did not pass", out=out[-2000:])
    rc, out = run_script("activate.sh", snap, "--yes-i-have-operator-approval", *deb_args(), log=f"activate-{name}")
    check(rc == 0 and "activate complete" in out, "activate failed", out=out[-3000:])
    return snap


def cgroup_procs(unit_name):
    path = Path(f"/sys/fs/cgroup/system.slice/{unit_name}/cgroup.procs")
    return set(path.read_text().split()) if path.exists() else set()


@case("16 measure.sh after activation (runbook step 5): report from the real /proc and cgroup tree, read-only")
def measure():
    before_c = containers()
    snap = activated("snapM")
    state["snapM"] = snap
    state["snapM_containers"] = before_c
    status, headers, _ = login("holly")
    check(status == 200, "login for the measurement session failed", status=status)
    name, value = headers["set-cookie"].split(";")[0].split("=", 1)
    cookie = Path("/root/measure.cookies")
    cookie.write_text(f"{HOST}\tFALSE\t/\tTRUE\t0\t{name}\t{value}\n")
    cookie.chmod(0o600)
    watched = [*RUST_UNITS, "limeos-storage-reader.service", "caddy.service", *FROZEN_UNITS]
    units_before = {u: unit(u) for u in watched}
    bystanders_before = bystanders()
    out_dir = Path("/var/lib/limeos-cutover/measure-rehearsal")
    try:
        rc, out = run_script("measure.sh", "--host", HOST, "--port", PORT, "--out", out_dir, "--window", 120, "--cookie-file", cookie, log="measure", timeout=900)
    finally:
        cookie.unlink(missing_ok=True)  # the runbook: delete it afterwards
    check(rc in (0, 2), "measure.sh failed to produce a report", rc=rc, out=out[-2500:])
    check(sorted(os.listdir(out_dir)) == ["raw.txt", "report.json"], "measure.sh wrote something else", files=os.listdir(out_dir))
    report = json.loads((out_dir / "report.json").read_text())
    rows = {m["id"]: m for m in report["measurements"]}
    check(report["read_only"] and not report["missing_units"] and not report["unknown_units"], "report has missing or unknown units", report={k: report[k] for k in ("read_only", "missing_units", "unknown_units")})
    measured = ("core_base_idle_pss", "core_base_idle_swap", "all_resident_idle_pss", "backend_cpu_idle", "resident_disk_writes", "health_http_p95", "cached_observation_http_p95", "installed_footprint")
    check(all(rows[i]["status"] in ("pass", "fail") for i in measured), "a row that should be measured is not", rows={i: rows[i]["status"] for i in measured})
    check((rc == 0) == (report["verdict"] == "pass"), "exit status disagrees with the verdict", rc=rc, verdict=report["verdict"])
    # Every process it measured is a member of the unit's real cgroup.
    members = {u: cgroup_procs(u) for u in [*RUST_UNITS, "limeos-storage-reader.service"]}
    foreign = [p for p in report["processes"] if str(p["pid"]) not in members.get(p["unit"], set())]
    check(not foreign, "report names a process outside its unit's cgroup", foreign=foreign, members={u: sorted(m) for u, m in members.items()})
    check({p["unit"] for p in report["processes"]} >= set(RUST_UNITS) | {"limeos-storage-reader.service"}, "a running LimeOS unit was not measured", units=sorted({p["unit"] for p in report["processes"]}))
    # Read-only: nothing restarted, nothing changed.
    check({u: unit(u) for u in watched} == units_before, "measure.sh changed a unit", before=units_before)
    check(containers() == before_c and bystanders() == bystanders_before, "measure.sh changed a container or a bystander")
    check(hash_tree() == state["frozen_hashes"], "measure.sh changed a frozen-app file")
    misses = {i: {"value": m["value"], "target": m["target"], "note": m.get("note")} for i, m in rows.items() if m["status"] == "fail"}
    if misses:
        state["notes"].append(f"measure.sh targets missed on the amd64 guest: {misses}")
    return {"exit": rc, "verdict": report["verdict"], "rows": {i: (m["status"], m["value"], m["unit"]) for i, m in rows.items()}, "misses": misses, "processes": [(p["unit"], p["comm"], p["pss_bytes"], p.get("cpu_ticks_delta"), p.get("write_bytes_delta")) for p in report["processes"]], "elapsed_s": rows["backend_cpu_idle"].get("elapsed_s"), "since_start_percent": rows["backend_cpu_idle"].get("since_start_percent")}


@case("17 return.lock: a second return.sh is refused while the first holds the lock (first suspended), then the first completes")
def concurrent_return():
    snap = state["snapM"]
    record = snap / "return.lock"
    first = subprocess.Popen([str(CUTOVER / "return.sh"), str(snap)], stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    held = False
    deadline = time.monotonic() + 60
    while time.monotonic() < deadline and first.poll() is None:
        # The lock is taken by flock(1) on return.sh's fd, then return.sh records itself: wait for both.
        try:
            recorded = record.read_text().split()
        except OSError:
            recorded = []
        if lock_holders(record) and recorded[:2] == ["return", str(first.pid)]:
            os.kill(first.pid, signal.SIGSTOP)
            held = True
            # The stop is delivered asynchronously: wait until the process reports state T.
            fx.eventually(lambda: Path(f"/proc/{first.pid}/stat").read_text().rsplit(")", 1)[1].split()[0] == "T", timeout=5)
            break
        time.sleep(0.002)
    try:
        check(held, "the first return.sh never held return.lock while running", rc=first.poll())
        process_state = Path(f"/proc/{first.pid}/stat").read_text().rsplit(")", 1)[1].split()[0]
        rc2, out2 = run_script("return.sh", snap, log="return-concurrent-second")
    finally:
        if held:
            os.kill(first.pid, signal.SIGCONT)
        out1, _ = first.communicate(timeout=600)
        state["logs"]["return-concurrent-first"] = out1[-20000:] + f"\n[exit {first.returncode}]\n"
    check(process_state == "T", "the first return.sh was not suspended", state=process_state)
    check(rc2 == 1 and f"return.sh (pid {first.pid}) is running on this snapshot; wait for it to finish" in out2, "the second return.sh was not refused naming the first", out=out2)
    check(not re.search(r"^\+ ", out2, re.M), "the refused return.sh ran a command", out=out2)
    check(first.returncode == 0 and "return verified" in out1, "the first return.sh did not complete after resuming", problems=problems(out1), out=out1[-1500:])
    expect_frozen_serving("after the concurrent returns", snap)
    expect_world_equal("after the concurrent returns", state["snapM_containers"], state["baseline_bystanders"])
    return {"second": out2.strip().splitlines(), "first_tail": out1.strip().splitlines()[-3:]}


def held_activation(name):
    """A real activation stopped inside its package step: a dpkg pre-invoke hook sleeps, as a dpkg stuck on I/O
    would. The frozen writers are already stopped and disabled; activate.sh holds activate.lock in its scope."""
    HOLD_SCRIPT.write_text(f"#!/bin/sh\ntouch {HOLD_MARK}\nexec /bin/sleep 900\n")
    HOLD_SCRIPT.chmod(0o755)
    HOLD_MARK.unlink(missing_ok=True)
    rc, out, snap = snapshot_ready(name)
    check(rc == 0, "preflight did not pass", out=out[-2000:])
    HOLD_CONFIG.write_text(f"pre-invoke={HOLD_SCRIPT}\n")
    log = LOGS / f"activate-{name}.out"
    proc = subprocess.Popen([str(CUTOVER / "activate.sh"), str(snap), "--yes-i-have-operator-approval", *deb_args()], stdout=log.open("w"), stderr=subprocess.STDOUT)
    try:
        fx.eventually(lambda: HOLD_MARK.exists() or proc.poll() is not None, timeout=180)
    finally:
        HOLD_CONFIG.unlink(missing_ok=True)
    check(HOLD_MARK.exists() and proc.poll() is None, "activation did not stop in its package step", out=log.read_text()[-2500:])
    scope = (snap / "activation-scope").read_text().strip()
    holder = (snap / "activate.lock").read_text().split()
    procs = cgroup_procs(scope)
    check(active(scope) == "active" and len(holder) == 4 and holder[0] == "activate" and holder[3] == scope and holder[1] in procs, "activate.lock is not held by activate.sh inside its scope", holder=holder, procs=sorted(procs), scope=active(scope))
    check(active("pi-health.service") == "inactive" and enabled("pi-health.service") == "disabled", "the frozen app was not stopped before the package step")
    return snap, proc, scope, log, holder, procs


def settle_held(snap, proc, scope):
    """After a failed check: end what is left of a held activation and return to the frozen app for later cases."""
    if active(scope) == "active":
        sh(f"systemctl kill --kill-whom=all --signal=KILL {scope}", check_rc=False)
        fx.eventually(lambda: active(scope) != "active", timeout=30)
    if proc.poll() is None:
        proc.kill()
    if active("pi-health.service") != "active":
        run_script("return.sh", snap, log=f"return-{snap.name}-cleanup")


def expect_forced_aftermath(label, snap, scope, out, before_c, procs):
    check("stopped unexpectedly" not in out, f"{label}: return.sh stopped unexpectedly", out=out[-2500:])
    check(not (snap / "force-takeover").exists(), f"{label}: the takeover marker is left behind")
    check(active(scope) == "inactive" and not Path(f"/sys/fs/cgroup/system.slice/{scope}").exists(), f"{label}: the activation's scope or its cgroup is left", scope=active(scope))
    survivors = [p for p in procs if Path(f"/proc/{p}").exists() and "rehearsal-hold" in Path(f"/proc/{p}/cmdline").read_text(errors="replace") + sh(f"ps -o args= -p {p}", check_rc=False)[1]]
    check(not survivors and not sh("pgrep -f 'sleep 900'", check_rc=False)[1].strip(), f"{label}: a process of the activation survived", survivors=survivors)
    audit = sh("dpkg --audit", check_rc=False)[1].strip()
    package = sh("dpkg-query -W -f='${Status} ${Version}' limeos")[1].strip()
    check(not audit and package == f"install ok installed {state['version']}", f"{label}: dpkg is not clean after the kill", audit=audit, package=package)
    expect_frozen_serving(label, snap)
    expect_world_equal(label, before_c, state["baseline_bystanders"])
    return {"dpkg_audit": audit or "clean", "package": package}


@case("18 return.sh --force during a held activation: plain return refused; --force ends the scope through the real cgroup.kill, exit 0, frozen app serving, marker gone")
def forced_return():
    before_c = containers()
    snap, proc, scope, log, holder, procs = held_activation("snapF1")
    try:
        rc0, out0 = run_script("return.sh", snap, log="return-during-activation")
        check(rc0 == 1 and f"activate.sh (pid {holder[1]}) is running on this snapshot in scope {scope}" in out0, "a plain return.sh did not refuse naming the activation", out=out0)
        check(not re.search(r"^\+ ", out0, re.M) and active(scope) == "active", "the refused plain return.sh changed something", out=out0)
        rc, out = run_script("return.sh", snap, "--force", log="return-force", timeout=600)
        follower = proc.wait(timeout=60)
        check(rc == 0 and "return verified" in out, "return.sh --force did not complete", rc=rc, problems=problems(out), out=out[-1500:])
        check(f"killed every process in scope {scope} at once by writing 1 to /sys/fs/cgroup/system.slice/{scope}/cgroup.kill" in out, "the scope was not ended through cgroup.kill", out=out[-3000:])
        check(f"the activation's scope {scope} is now empty; the forced takeover" in out, "the takeover was not reported complete", out=out[-3000:])
        check("stopped unexpectedly" not in log.read_text(), "the killed activation's follower reported an unexpected stop", tail=log.read_text()[-600:])
        detail = expect_forced_aftermath("after a forced return", snap, scope, out, before_c, procs)
    except BaseException:
        settle_held(snap, proc, scope)
        raise
    return {**detail, "scope": scope, "plain_refusal": out0.strip().splitlines()[-1:], "force_seconds": state["timings"]["return-force"], "activate_follower_exit": follower, "activate_log_tail": log.read_text().strip().splitlines()[-4:], "force_output": out.strip().splitlines()[:8]}


@case("19 return.sh --force under strace: after the kill it reads the removed scope cgroup's cgroup.events (ENOENT) on bash 5.2 and carries on")
def forced_return_traced():
    before_c = containers()
    snap, proc, scope, log, holder, procs = held_activation("snapF2")
    trace = Path("/root/return-force.strace")
    try:
        bash = sh("bash --version | head -1")[1].strip()
        started = time.monotonic()
        rc, out = sh(f"strace -f -qq -e trace=openat -o {trace} {CUTOVER}/return.sh {snap} --force", check_rc=False, timeout=600, log="return-force-strace")
        state["timings"]["return-force-strace"] = round(time.monotonic() - started, 1)
        proc.wait(timeout=60)
        events = [line for line in trace.read_text().splitlines() if f"/sys/fs/cgroup/system.slice/{scope}/cgroup.events" in line]
        check(rc == 0 and "return verified" in out, "return.sh --force did not complete under strace", rc=rc, problems=problems(out), out=out[-1500:])
        check(any("ENOENT" in line for line in events), "the removed cgroup's cgroup.events was never read (path not exercised)", events=events)
        detail = expect_forced_aftermath("after a traced forced return", snap, scope, out, before_c, procs)
    except BaseException:
        settle_held(snap, proc, scope)
        raise
    return {**detail, "bash": bash, "cgroup_events_reads": events[-6:]}


@case("20 return.sh --force once the scope is gone (ended by hand with the runbook's systemctl kill): same result, exit 0, frozen app serving")
def forced_return_scope_gone():
    before_c = containers()
    snap, proc, scope, log, holder, procs = held_activation("snapF3")
    try:
        sh(f"systemctl kill --kill-whom=all --signal=KILL {scope}", log="manual-scope-kill")
        fx.eventually(lambda: active(scope) == "inactive", timeout=30)
        follower = proc.wait(timeout=60)
        gone = not Path(f"/sys/fs/cgroup/system.slice/{scope}").exists()
        check(gone, "the scope's cgroup still exists after the manual kill")
        rc, out = run_script("return.sh", snap, "--force", log="return-force-scope-gone", timeout=600)
        check(rc == 0 and "return verified" in out, "return.sh --force did not complete once the scope was gone", rc=rc, problems=problems(out), out=out[-1500:])
        check("killed every process" not in out, "return.sh --force killed something although the scope was gone", out=out[-2000:])
        detail = expect_forced_aftermath("after a forced return with the scope gone", snap, scope, out, before_c, procs)
    except BaseException:
        settle_held(snap, proc, scope)
        raise
    return {**detail, "activate_follower_exit": follower, "output_head": out.strip().splitlines()[:6]}


def exposure(text):
    found = re.search(r"Overall exposure level for \S+: (\d+)\.(\d)", text)
    return int(found[1]) * 10 + int(found[2]) if found else None


@case("21 unit hardening on systemd 252: shipped units start, systemd-analyze security within the ceilings, core Type=notify with a live watchdog")
def hardening():
    spec = importlib.util.spec_from_file_location("check_unit_exposure", SRC / "scripts/check_unit_exposure.py")
    gate = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(gate)
    # LimeOS runs only while activated on a host whose frozen app owns /var/lib/limeos.
    snap = activated("snapH")
    installed = sorted(p.name for p in Path("/lib/systemd/system").glob("limeos-*.service"))
    check(installed == sorted(SHIPPED_UNITS) == sorted(gate.CEILINGS), "shipped units differ from the gate's list", installed=installed, ceilings=sorted(gate.CEILINGS))
    # What starts without storage hardware: the reader (activate.sh starts it) and the targets executor under a
    # deny-all ceiling. storage-ready waits for real disks; storage-mounts needs the storage contract.
    policy = Path("/etc/limeos/system-policy/storage-targets.json")
    policy.write_text(json.dumps({"version": 1, "core_uid": pwd.getpwnam("limeos-core").pw_uid, "allow_prepare_targets": False, "managed_targets": []}) + "\n")
    policy.chmod(0o644)
    extra = ["limeos-storage-reader.service", "limeos-storage-targets.service"]
    try:
        started = time.monotonic()
        starts = {u: sh(f"systemctl start {u}", check_rc=False) for u in extra}
        core1 = unit("limeos-core.service", "Type", "NotifyAccess", "WatchdogUSec", "StatusText", "WatchdogTimestampMonotonic", "MainPID", "NRestarts")
        watch1 = {u: unit(u, "WatchdogUSec", "WatchdogTimestampMonotonic") for u in SHIPPED_UNITS[:5]}
        scores = {}
        for u in SHIPPED_UNITS:
            online = sh(f"systemd-analyze security --no-pager {u}", check_rc=False)[1]
            offline = sh(f"systemd-analyze security --offline=true --no-pager /lib/systemd/system/{u}", check_rc=False)[1]
            scores[u] = {"online": exposure(online), "offline": exposure(offline), "ceiling": gate.CEILINGS[u]}
            state["logs"][f"exposure-{u}"] = online[-6000:]
        time.sleep(max(0.0, 36 - (time.monotonic() - started)))  # past one 30 s watchdog interval
        core2 = unit("limeos-core.service", "Type", "NotifyAccess", "WatchdogUSec", "StatusText", "WatchdogTimestampMonotonic", "MainPID", "NRestarts")
        watch2 = {u: unit(u, "WatchdogUSec", "WatchdogTimestampMonotonic") for u in SHIPPED_UNITS[:5]}
        units = {u: unit(u, "ActiveState", "SubState", "Result", "NRestarts", "ConditionResult", "MainPID") for u in SHIPPED_UNITS}
    finally:
        for u in extra:
            if u != "limeos-storage-reader.service":
                sh(f"systemctl stop {u}", check_rc=False)
        policy.unlink(missing_ok=True)
        rc_return, out_return = run_script("return.sh", snap, log="return-after-hardening")
    running = SHIPPED_UNITS[:5]
    check(all(starts[u][0] == 0 for u in extra), "a unit failed to start", starts={u: starts[u][1][-400:] for u in extra})
    check(all(units[u]["ActiveState"] == "active" and units[u]["NRestarts"] == "0" for u in running), "a started unit is not active or has restarted", units=units)
    missing = [u for u, s in scores.items() if s["online"] is None]
    check(not missing, "systemd-analyze gave no score", units=missing)
    above = {u: s for u, s in scores.items() if s["online"] > s["ceiling"]}
    check(core2["Type"] == "notify" and core2["NotifyAccess"] == "main" and core2["WatchdogUSec"] == "30s" and core2["StatusText"] == "Serving", "core's notify settings or status differ", core=core2)
    check(int(core2["WatchdogTimestampMonotonic"]) > int(core1["WatchdogTimestampMonotonic"]) > 0 and core2["MainPID"] == core1["MainPID"], "core's watchdog keep-alives are not arriving", before=core1, after=core2)
    pinged = {u: int(watch2[u]["WatchdogTimestampMonotonic"]) > int(watch1[u]["WatchdogTimestampMonotonic"]) for u in running if watch2[u]["WatchdogUSec"] not in ("0", "infinity")}
    check(all(pinged.values()), "a watchdog-armed unit sent no keep-alive", pinged=pinged, before=watch1, after=watch2)
    check(not above, "a unit's exposure is above its ceiling on systemd 252", above=above, margin=gate.MARGIN)
    check(rc_return == 0 and "return verified" in out_return, "return.sh after the hardening case failed", problems=problems(out_return))
    systemd = sh("systemctl --version | head -1")[1].strip()
    return {"systemd": systemd, "scores_tenths": scores, "core": core2, "watchdog_pinged": pinged, "not_started": {u: units[u] for u in SHIPPED_UNITS[5:]}}


def journal_stops(since):
    """limeos-* stop jobs systemd completed since SINCE (epoch seconds), in order."""
    out = sh(f"journalctl -o json --no-pager --since @{int(since)} _PID=1", timeout=60)[1]
    stops = []
    for line in out.splitlines():
        try:
            entry = json.loads(line)
        except json.JSONDecodeError:
            continue
        name = entry.get("UNIT", "")
        if name.startswith("limeos-") and entry.get("JOB_TYPE") == "stop" and entry.get("JOB_RESULT") == "done":
            stops.append((int(entry["__MONOTONIC_TIMESTAMP"]), name, entry.get("JOB_RESULT"), entry.get("MESSAGE")))
    return [s[1:] for s in sorted(stops)]


def package_data():
    policies = {p: sha(p) for p in sorted(glob.glob("/etc/limeos/system-policy/*.json"))}
    return {"users": users(), "policies": policies, "core.json": sha("/etc/limeos/core.json"), "catalog": sha(CATALOG), "frozen": hash_tree()}


@case("22 maintainer scripts on real dpkg during an activation: remove stops every unit in order and keeps state; second remove no-op; reinstall restarts; upgrade to 0.4.17+cutover.2 keeps services and data")
def package_lifecycle():
    before_c = containers()
    snap = activated("snapP")
    check(active("limeos-storage-reader.service") == "active", "the reader is not running after activation")
    apt = "DEBIAN_FRONTEND=noninteractive apt-get"
    data = package_data()
    started = time.time() - 1
    check(all(Path(f"/etc/systemd/system/{u}.d/{GUARD}").exists() for u in CLAIMANTS), "the activation's guards are missing")
    rc, out = sh(f"{apt} remove -y limeos", check_rc=False, timeout=600, log="apt-remove-1")
    check(rc == 0, "apt-get remove failed", out=out[-2500:])
    # LimeOS removed by hand cannot leave the frozen app unstartable: postrm deletes the guards.
    check(not any(Path(f"/etc/systemd/system/{u}.d/{GUARD}").exists() for u in CLAIMANTS) and "removed the cutover guard" in out, "removal left a cutover guard", out=out[-1500:])
    stops = journal_stops(started)
    order_seen = [u for u, *_ in stops]
    check(order_seen == ["limeos-core.service", "limeos-containerd.service", "limeos-storaged.service", "limeos-storage-reader.service"], "units were not stopped core, executors, reader", stops=stops)
    check(all(active(u) in ("inactive", "failed") for u in SHIPPED_UNITS) and all(not Path(f"/sys/fs/cgroup/system.slice/{u}").exists() for u in SHIPPED_UNITS), "a unit or its cgroup survived the removal")
    check("survived" not in out and "Cannot tell" not in out and "Warning" not in out, "prerm reported a problem", out=out[-2500:])
    status = sh("dpkg-query -W -f='${Status}' limeos")[1].strip()
    links = [p.name for p in Path("/etc/systemd/system/multi-user.target.wants").glob("limeos-*") if p.name in SHIPPED_UNITS]
    check(status == "deinstall ok config-files" and not links, "package not removed cleanly", status=status, links=links)
    check(Path("/var/lib/limeos/core/core.sqlite").exists() and package_data() == data, "removal changed /var/lib/limeos or /etc/limeos")
    # A second removal is a no-op.
    rc2, out2 = sh(f"{apt} remove -y limeos", check_rc=False, timeout=300, log="apt-remove-2")
    rc3, out3 = sh("dpkg --remove limeos", check_rc=False, timeout=300, log="dpkg-remove-2")
    check(rc2 == 0 and "is not installed, so not removed" in out2 and rc3 == 0, "a second removal was not a no-op", apt=out2[-800:], dpkg=out3[-800:])
    check(journal_stops(started) == stops, "a second removal stopped something", stops=journal_stops(started))
    check(package_data() == data and sh("dpkg-query -W -f='${Status}' limeos")[1].strip() == status, "a second removal changed something")
    # Reinstall: configure starts what the package starts (the reader's restart record went with the removal).
    rc4, out4 = sh(f"{apt} install -y limeos={state['version']}", check_rc=False, timeout=600, log="apt-reinstall")
    check(rc4 == 0, "reinstall failed", out=out4[-2500:])
    check(all(active(u) == "active" and enabled(u) == "enabled" for u in RUST_UNITS), "reinstall did not start and enable the core units", units={u: (active(u), enabled(u)) for u in RUST_UNITS})
    check(active("limeos-storage-reader.service") == "inactive", "reinstall started the reader")
    check(sh("/usr/lib/limeos/limeosctl --socket /run/limeos-core/core.sock status", check_rc=False)[0] == 0, "core not ready after reinstall")
    check(package_data() == data, "reinstall changed state or configuration")
    check(fx.eventually(lambda: login("holly")[0] == 200, timeout=30), "login through the proxy failed after reinstall")
    # Upgrade to a second build: the reader runs again (as after activation or the runbook's manual start).
    sh("systemctl start limeos-storage-reader.service")
    pids = {u: unit(u)["MainPID"] for u in [*RUST_UNITS, "limeos-storage-reader.service"]}
    data = package_data()
    rc5, out5 = sh(f"{apt} install -y limeos={UPGRADE_VERSION}", check_rc=False, timeout=600, log="apt-upgrade")
    check(rc5 == 0, "upgrade failed", out=out5[-2500:])
    version = sh("dpkg-query -W -f='${Status} ${Version}' limeos")[1].strip()
    check(version == f"install ok installed {UPGRADE_VERSION}", "upgrade not installed", version=version)
    check("giving every entry of LimeOS's trees" not in out5, "an ordinary upgrade reclaimed LimeOS's trees", out=out5[-1500:])
    after = {u: (active(u), enabled(u), unit(u)["MainPID"]) for u in [*RUST_UNITS, "limeos-storage-reader.service"]}
    check(all(after[u][0] == "active" and after[u][1] == "enabled" for u in RUST_UNITS) and after["limeos-storage-reader.service"][0] == "active", "a service is not running after the upgrade", after=after)
    check(all(after[u][2] != pids[u] for u in pids), "the upgrade did not restart every running service", before=pids, after=after)
    check(not Path("/run/limeos-package").exists(), "the restart record outlived the upgrade")
    check(package_data() == data, "the upgrade changed state or configuration")
    check(login("holly")[0] == 200, "login through the proxy failed after the upgrade")
    # An interrupted package change finished by `dpkg --configure -a` while the activation's guards are in place
    # (they were deleted with the removal above; put them back as activate.sh writes them).
    for name in CLAIMANTS:
        Path(f"/etc/systemd/system/{name}.d").mkdir(exist_ok=True)
        Path(f"/etc/systemd/system/{name}.d/{GUARD}").write_text(f"[Unit]\nAssertPathExists=!/etc/systemd/system/{name}.d/{GUARD}\n")
    sh("systemctl daemon-reload")
    upgrade_deb = glob.glob(f"/opt/limeos-repo/**/limeos_{UPGRADE_VERSION}_*.deb", recursive=True)[0]
    sh(f"dpkg --unpack {upgrade_deb}", timeout=300, log="dpkg-unpack-under-guard")
    check("unpacked" in sh("dpkg-query -W -f='${Status}' limeos")[1], "the unpack did not leave the package unconfigured")
    rc7, out7 = sh("DEBIAN_FRONTEND=noninteractive dpkg --configure -a", check_rc=False, timeout=600, log="dpkg-configure-a-under-guard")
    check(rc7 == 0 and sh("dpkg-query -W -f='${Status}' limeos")[1].strip() == "install ok installed", "dpkg --configure -a did not finish the package", out=out7[-1500:])
    check(all(active(u) == "active" for u in RUST_UNITS) and owner_mode("/var/lib/limeos") == (0, 0, 0o755), "configure under the guard did not start LimeOS in the taken-over directory")
    check(sh("systemctl start pi-health.service", check_rc=False)[0] != 0 and active("pi-health.service") != "active", "the guard did not hold after dpkg --configure -a")
    # Back to the frozen app; the upgraded package stays installed.
    rc6, out6 = run_script("return.sh", snap, log="return-after-package")
    check(rc6 == 0 and "return verified" in out6, "return.sh after the package changes failed", problems=problems(out6), out=out6[-1500:])
    expect_frozen_serving("after the package changes", snap)
    expect_world_equal("after the package changes", before_c, state["baseline_bystanders"])
    return {"stops": stops, "second_remove": out2.strip().splitlines()[-2:], "dpkg_second_remove": out3.strip().splitlines()[-2:], "after_upgrade": after, "seconds": {k: state["timings"].get(k) for k in ("activate-snapP", "return-after-package")}}


@case("22b package change while the frozen app owns /var/lib/limeos: configure leaves it and LimeOS's units alone, says so on stderr and in the journal")
def package_change_elsewhere():
    before_c = containers()
    apt = "DEBIAN_FRONTEND=noninteractive apt-get"
    started = time.strftime("%Y-%m-%d %H:%M:%S")
    rc, out = sh(f"{apt} install -y --allow-downgrades limeos={state['version']}", check_rc=False, timeout=600, log="apt-downgrade-elsewhere")
    check(rc == 0, "the package change failed", out=out[-2500:])
    check("/var/lib/limeos belongs to holly:pihealth (750)" in out and "neither enabled nor started" in out, "configure did not say it left LimeOS stopped", out=out[-1500:])
    journal = sh(f"journalctl -t limeos-postinst --since '{started}' --no-pager", check_rc=False)[1]
    check("neither enabled nor started" in journal, "the notice is not in the journal", journal=journal[-1000:])
    check(all(active(u) in ("inactive", "failed") and enabled(u) == "disabled" for u in RUST_UNITS), "configure started or enabled a LimeOS unit", units={u: (active(u), enabled(u)) for u in RUST_UNITS})
    expect_frozen_serving("after a package change while the frozen app owns the directory")
    expect_world_equal("after a package change while the frozen app owns the directory", before_c, state["baseline_bystanders"])
    return {"notice": [l for l in out.splitlines() if l.startswith("LimeOS")], "journal": journal.strip().splitlines()[-3:]}


@case("23 PR-32: a reader run as root by a leftover drop-in is refused by core (reader-status, degraded, journal); with the drop-in removed, a package upgrade brings it back as limeos-storage-reader and core's reads turn fresh; activation itself refuses a root reader and rolls back")
def reader_root_dropin():
    """The production host's earlier package ran the reader as root. A real upgrade from that package
    is not available in this guest (one candidate and its relabelled second build are), so the root
    reader is reproduced with a drop-in on the candidate: what the upgrade must cope with is the same
    (a running root reader that prerm stops, its root-owned runtime directory, a postinst that starts
    it again as the account). What it cannot reproduce is the old package's own prerm and binaries."""
    from p04_fixture_readiness import READER_ACCOUNT, READER_SOCKET, reader_confined, reader_status

    ctl = "/usr/lib/limeos/limeosctl --socket /run/limeos-core/core.sock"
    reader = "limeos-storage-reader.service"
    account = pwd.getpwnam(READER_ACCOUNT).pw_uid
    dropin_dir = Path(f"/etc/systemd/system/{reader}.d")
    dropin = dropin_dir / "zz-pr32-root.conf"
    apt = "DEBIAN_FRONTEND=noninteractive apt-get"
    facts = {}

    def place_dropin():
        dropin_dir.mkdir(exist_ok=True)
        dropin.write_text("[Service]\nUser=root\n")
        sh("systemctl daemon-reload")

    def remove_dropin():
        dropin.unlink(missing_ok=True)
        if dropin_dir.exists() and not any(dropin_dir.iterdir()):
            dropin_dir.rmdir()
        sh("systemctl daemon-reload")

    def runs_as_account():
        pid = int(unit(reader)["MainPID"])
        return (
            active(reader) == "active"
            and pid > 1
            and READER_SOCKET.is_socket()
            and READER_SOCKET.lstat().st_uid == account
            and Path(f"/proc/{pid}").stat().st_uid == account
            and reader_confined(reader_status(pid))
        )

    snap = activated("snapR")
    check(runs_as_account(), "the reader is not limeos-storage-reader after activation")
    try:
        place_dropin()
        started = time.strftime("%Y-%m-%d %H:%M:%S")
        sh(f"systemctl restart {reader}")
        check(fx.eventually(lambda: READER_SOCKET.is_socket() and READER_SOCKET.lstat().st_uid == 0, timeout=20), "the drop-in did not run the reader as root")
        rc, out = sh(f"{ctl} reader-status", check_rc=False)
        check(rc != 0 and '"state":"refused"' in out, "core did not refuse a root-owned reader socket", rc=rc, out=out)
        facts["refused"] = out.strip()

        def listed():
            health = json.loads(sh(f"{ctl} status")[1])
            return any(d["subsystem"] == "storage_reader" and (d.get("detail") or {}).get("code") == "forbidden" for d in health.get("degraded", []))

        check(fx.eventually(listed, timeout=30), "core's health does not list the refused reader")
        journal = sh(f"journalctl -u limeos-core --since '{started}' --no-pager", check_rc=False)[1]
        check("core refuses the storage reader's socket" in journal, "core did not log the refusal", journal=journal[-1500:])
        # The upgrade from a root reader: the drop-in goes, the package is upgraded with the root reader running.
        remove_dropin()
        rc, out = sh(f"{apt} install -y --allow-downgrades limeos={UPGRADE_VERSION}", check_rc=False, timeout=600, log="apt-upgrade-root-reader")
        check(rc == 0, "the upgrade from a root reader failed", out=out[-2500:])
        check(fx.eventually(runs_as_account, timeout=30), "the reader did not come back as limeos-storage-reader after the upgrade", status=reader_status(int(unit(reader)["MainPID"])) if int(unit(reader)["MainPID"]) > 1 else None)
        rc, out = sh(f"{ctl} reader-status", check_rc=False)
        check(rc == 0 and '"state":"accepted"' in out, "core does not accept the reader after the upgrade", out=out)
        status, headers, view = login("holly")
        check(status == 200, "login after the upgrade failed", status=status)
        cookie = headers["set-cookie"].split(";")[0]

        def fresh():
            code, _, value = https("/api/v1/deployment/stacks", cookie=cookie)
            return code == 200 and value["catalog"]["state"] == "fresh" and value["engine"]["state"] == "fresh"

        check(fx.eventually(fresh, timeout=90), "core's stack status did not turn fresh from the account reader", stacks=str(https("/api/v1/deployment/stacks", cookie=cookie)[2])[:1000])
        code, _, inventory = https("/api/v1/storage/inventory", cookie=cookie)
        check(code == 200 and inventory["inventory"]["devices"], "core's storage inventory from the account reader failed", code=code, body=str(inventory)[:800])
        facts["devices"] = [(d["filesystem"], d["boot_backing"]) for d in inventory["inventory"]["devices"]]
    finally:
        remove_dropin()
    rc, out = run_script("return.sh", snap, log="return-after-root-reader")
    check(rc == 0 and "return verified" in out, "return.sh after the root-reader upgrade failed", problems=problems(out), out=out[-1500:])
    # Activation refuses a reader a drop-in runs as root, names the drop-in, and rolls back.
    place_dropin()
    try:
        facts["failed_activation"] = failed_activation("reader-root", "snapR2", [], "is owned by root, not limeos-storage-reader", version=None)
        findings = Path("/var/lib/limeos-cutover/snapR2/findings.txt").read_text()
        check(f"LimeOS unit drop-in" in findings and str(dropin) in findings, "preflight did not name the reader's drop-in", findings=findings[-1200:])
    finally:
        remove_dropin()
    return facts


# --- needs the merged build (production-readiness with pr-health-cohort) ------------------------------------
# wybie's container mix (26 Nov host facts): 18 containers, 14 running, health checks on some, 51 mounts in all.
# The 0.4.4 reader failed core's container-health read on that host (`the read failed: Unavailable`) and
# activation rolled back; the fix is on branch pr-health-cohort. On binaries without it this case is expected
# to FAIL at the first activation, and every earlier case is independent of it (it runs last, and only adds).
COHORT_RUNNING = ["sabnzbd", "jackett", "navidrome", "audiobookshelf", "rdtclient", "requestrr", "get_iplayer", "yt-to-jellyfin"]
COHORT_HEALTHY = ["sabnzbd", "jackett", "navidrome"]
COHORT_FAILED_CHECK = ["limeos-mattermost", "limeos-alertd"]
COHORT_MOUNTS = 51


def mount_count():
    return sum(len(json.loads(sh(f"docker inspect {n}")[1])[0]["Mounts"]) for n in sh("docker ps -a --format '{{.Names}}'")[1].split())


def health(name):
    return json.loads(sh(f"docker inspect {name}")[1])[0]["State"].get("Health", {}).get("Status")


def storage_ceiling(grant):
    """Set allow_container_health_read in the reader's ceiling, keeping root:limeos-host-access 0640."""
    path = Path("/etc/limeos/system-policy/storage.json")
    value = json.loads(path.read_text())
    value["allow_container_health_read"] = grant
    temporary = path.with_name(".storage.json.new")
    temporary.write_text(json.dumps(value) + "\n")
    sh(f"chown root:limeos-host-access {temporary} && chmod 0640 {temporary} && mv -f {temporary} {path}")
    check(permitted(path) == "root:limeos-host-access 640", "storage.json lost its owner or mode", mode=permitted(path))


def reads():
    """{read: (exit, answer)} for core's three reads through the stack reader."""
    ctl = "/usr/lib/limeos/limeosctl --socket /run/limeos-core/core.sock"
    return {r: sh(f"{ctl} reader-status --read {r}", check_rc=False) for r in ("stack", "storage", "container-health")}


@case("24 NEEDS THE MERGED BUILD (pr-health-cohort): wybie's mix of 18 containers (14 running, 3 healthy checks, 2 stopped after a failed check, 51 mounts): activation passes with all three reader reads accepted; with allow_container_health_read false the read is not_granted (exit 0) and activation still passes")
def health_cohort():
    managed = Path("/root/managed.txt")
    facts = {}
    # The fixture grows to wybie's mix: eight more running media members (three with passing health checks),
    # the refused outside project's container goes (wybie has 4 stopped), and two mattermost members are
    # re-created with a check that fails, then stopped, as on the host.
    sh("docker rm -f airsonic-old", check_rc=False)
    existing = mount_count()
    new = [*COHORT_RUNNING, *COHORT_FAILED_CHECK]
    per, extra = divmod(COHORT_MOUNTS - existing, len(new))
    check(per >= 1, "too many mounts already", existing=existing)
    for index, name in enumerate(new):
        if name in COHORT_FAILED_CHECK:
            sh(f"docker rm -f {name}")
        binds = ""
        for k in range(per + (1 if index < extra else 0)):
            directory = Path(f"/srv/cohort/{name}/{k}")
            directory.mkdir(parents=True, exist_ok=True)
            binds += f"-v {directory}:/data/{k} "
        project = "mattermost" if name in COHORT_FAILED_CHECK else "media"
        labels = (f"--label com.docker.compose.project={project} --label com.docker.compose.service={name} "
                  f"--label com.docker.compose.project.working_dir=/opt/stacks/{project} "
                  f"--label com.docker.compose.project.config_files=/opt/stacks/{project}/compose.yaml --label com.docker.compose.oneoff=False ")
        if name in COHORT_HEALTHY:
            check_flags = "--health-cmd '/bin/busybox true' --health-interval 3s --health-retries 2 "
        elif name in COHORT_FAILED_CHECK:
            check_flags = "--health-cmd '/bin/busybox false' --health-interval 2s --health-retries 1 "
        else:
            check_flags = ""
        command = f"echo {name} ready; trap 'exit 0' TERM INT; while :; do /bin/busybox sleep 1; done"
        state["ids"][name] = sh(f"runuser -u holly -- docker run -d --name {name} --restart unless-stopped {labels}{check_flags}{binds}{IMAGE} /bin/busybox sh -c \"{command}\"")[1].strip()
    # The media Compose file names its new members, and the catalog is proposed and installed again, as the
    # runbook says to whenever a Compose file under /opt/stacks changes.
    compose = Path("/opt/stacks/media/compose.yaml")
    compose.write_text(compose.read_text() + "".join(f"  {n}:\n    image: x\n" for n in COHORT_RUNNING))
    Path("/root/management-catalog.cohort.json").unlink(missing_ok=True)
    sh("/usr/lib/limeos/limeosctl migrate propose-catalog --out /root/management-catalog.cohort.json", log="propose-catalog-cohort")
    shutil_copy("/root/management-catalog.cohort.json", CATALOG)
    fx.eventually(lambda: all(health(n) == "healthy" for n in COHORT_HEALTHY), timeout=60)
    fx.eventually(lambda: all(health(n) == "unhealthy" for n in COHORT_FAILED_CHECK), timeout=60)
    sh("runuser -u holly -- docker stop -t 1 " + " ".join(COHORT_FAILED_CHECK))
    c = containers()
    running = sorted(n for n, v in c.items() if v["running"])
    facts["mix"] = {"containers": len(c), "running": len(running), "stopped": sorted(n for n, v in c.items() if not v["running"]),
                    "mounts": mount_count(), "health": {n: health(n) for n in [*COHORT_HEALTHY, *COHORT_FAILED_CHECK]}}
    check(len(c) == 18 and len(running) == 14 and facts["mix"]["mounts"] == COHORT_MOUNTS, "the fixture is not wybie's mix", mix=facts["mix"])
    try:
        managed.write_text("\n".join(running) + "\n")
        out_dir = "/var/lib/limeos-cutover/snapC1"
        rc, out = run_script("preflight.sh", *preflight_args(), "--out", out_dir, "--managed-file", managed, log="preflight-snapC1")
        check(rc == 0 and "preflight passed" in out, "preflight did not pass on the 18-container mix", out=out[-2000:])
        rc, out = run_script("activate.sh", out_dir, "--yes-i-have-operator-approval", *deb_args(), log="activate-snapC1", timeout=600)
        check(rc == 0 and "activate complete" in out and "ROLLBACK" not in out, "activation rolled back on wybie's container mix", problems=problems(out), out=out[-2500:])
        granted = reads()
        facts["granted"] = {r: (rc_, a.strip()[-300:]) for r, (rc_, a) in granted.items()}
        check(all(rc_ == 0 and '"result":"accepted"' in a for rc_, a in granted.values()), "a reader read was not accepted", reads=facts["granted"])
        rc, out = run_script("return.sh", out_dir, log="return-snapC1")
        check(rc == 0 and "return verified" in out, "return.sh after the cohort activation failed", problems=problems(out))
        # The read the ceiling does not grant: not_granted with exit 0, and activation tolerates it.
        storage_ceiling(False)
        out_dir = "/var/lib/limeos-cutover/snapC2"
        rc, out = run_script("preflight.sh", *preflight_args(), "--out", out_dir, "--managed-file", managed, log="preflight-snapC2")
        check(rc == 0, "preflight did not pass", out=out[-2000:])
        rc, out = run_script("activate.sh", out_dir, "--yes-i-have-operator-approval", *deb_args(), log="activate-snapC2", timeout=600)
        check(rc == 0 and "activate complete" in out and "ROLLBACK" not in out, "activation did not tolerate a container-health read the ceiling does not grant", problems=problems(out), out=out[-2500:])
        rc_read, answer = reads()["container-health"]
        facts["not_granted"] = (rc_read, answer.strip()[-300:])
        check(rc_read == 0 and '"result":"not_granted"' in answer, "the ungranted container-health read is not not_granted with exit 0", answer=answer[-600:], rc=rc_read)
        rc, out = run_script("return.sh", out_dir, log="return-snapC2")
        check(rc == 0 and "return verified" in out, "return.sh after the not_granted activation failed", problems=problems(out))
    finally:
        storage_ceiling(True)
        if active("pi-health.service") != "active":
            run_script("return.sh", "/var/lib/limeos-cutover/snapC2" if Path("/var/lib/limeos-cutover/snapC2/meta.env").exists() else "/var/lib/limeos-cutover/snapC1", log="return-cohort-cleanup")
    expect_frozen_serving("after the cohort case")
    return facts


# ---------------------------------------------------------------- driver
def main(package_version="0.4.17+cutover.1"):
    if os.getuid() != 0 or socket.gethostname() != "limeos-p01-test":
        raise SystemExit("Disposable guest required")
    repository, output = map(Path, sys.argv[1:])
    started = time.time()
    setup(repository, package_version)
    for run in order:
        run()
    document = {
        "summary": "pass" if all(r["status"] == "pass" for r in results.values()) else "fail",
        "package_version": package_version,
        "cases": results,
        "notes": state["notes"],
        "facts": {
            "docker": state.get("docker_info"),
            "caddy": state.get("caddy_version"),
            "docker_socket_peer_pid_uid_gid": state.get("docker_socket_peer_pid_uid_gid"),
            "page_size": sh("getconf PAGESIZE")[1].strip(),
            "kernel": sh("uname -r")[1].strip(),
            "setup_seconds": round(time.time() - started, 1),
            "binary_sha256": {n: sha(f"/usr/lib/limeos/{n}") if Path(f"/usr/lib/limeos/{n}").exists() else None for n in ("limeos-core", "limeos-executor", "limeosctl", "limeos-password-worker")},
            "script_sha256": {n: sha(CUTOVER / n) for n in ("lib.sh", "preflight.sh", "activate.sh", "return.sh", "measure.sh", "state_lists.py", "Caddyfile.cutover.example")},
            "runbook_sha256": sha(RUNBOOK),
            "bash": sh("bash --version | head -1")[1].strip(),
            "systemd": sh("systemctl --version | head -1")[1].strip(),
            "timings_seconds": state["timings"],
        },
        "logs": state["logs"],
    }
    encoded = json.dumps(document, indent=2, default=str) + "\n"
    output.write_text(encoded)
    summary = {k: v["status"] for k, v in results.items()}
    print("CUTOVER_RESULT_BEGIN\n" + json.dumps(summary, indent=1) + "\nCUTOVER_RESULT_END", flush=True)
    if document["summary"] != "pass":
        raise SystemExit(1)


if __name__ == "__main__":
    main()
