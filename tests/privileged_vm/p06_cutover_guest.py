#!/usr/bin/env python3
"""Basic-cutover rehearsal; exclusively inside the disposable Debian VM that imitates wybie.

Runs the real scripts/cutover/{preflight,activate,return}.sh and `limeosctl migrate` against:
  * an imitated frozen app: pi-health.service (HTTP on 0.0.0.0:8002 as `holly`), pihealth-helper.service
    (PartOf), the metrics-collector and package-reconcile timers (the collector writes a SQLite file),
    /etc/limeos/credentials.env with a real Werkzeug scrypt and PBKDF2 hash, state JSON/SQLite files;
  * bystander units limeops-*, limeos-agent, flask-shutdown that must never change;
  * the packaged Docker Engine (socket-activated, as on a stock host), seven real containers created by
    `holly` (compose project `media` with four running members and one stopped, plus two standalone),
    /opt/stacks owned by holly;
  * Caddy from Debian, installed but inactive, serving the cutover site on 8002 with its internal CA.
Each case records pass/fail and continues. The full result is printed between CUTOVER_RESULT markers.
"""

import glob
import hashlib
import json
import os
import pwd
import re
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
CUTOVER = SRC / "scripts/cutover"
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
MEDIA_RUNNING = ["jellyfin", "radarr", "sonarr", "transmission"]
METRICS_DB = Path("/var/lib/limeos/metrics/metrics.sqlite")

results = {}
order = []
state = {"frozen_hashes": {}, "logs": {}, "notes": [], "session": {}}


# ---------------------------------------------------------------- helpers
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
    """sha256 of every frozen-app file recorded at fixture time (SQLite excluded; see sqlite_facts)."""
    return {p: sha(p) for p in state["frozen_files"] if Path(p).exists()}


def sqlite_facts():
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
    rc, out = sh(command, check_rc=check_rc, timeout=timeout, log=log)
    return rc, out


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
db = sqlite3.connect("/var/lib/limeos/metrics/metrics.sqlite")
db.execute("create table if not exists samples (t integer, load real)")
db.execute("insert into samples values (?, ?)", (int(time.time()), 0.1))
db.commit()
'''


def unit_file(name, text):
    Path(f"/etc/systemd/system/{name}").write_text(text)


def write_fixture(version_hashes):
    sh("useradd -m -s /bin/bash holly")
    sh("apt-get update -qq", timeout=300)
    sh("DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends docker.io busybox-static caddy sqlite3 iproute2 curl", timeout=900, log="apt-fixture")
    sh("usermod -aG docker holly")
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
    for d in ("agent-actions", "integrations", "metrics"):
        Path(f"/var/lib/limeos/{d}").mkdir(parents=True)
    for name in ("auto_update.json", "backup.json", "package-approvals.json", "pending-actions.json"):
        Path(f"/var/lib/limeos/{name}").write_text('{"frozen": true}\n')
    Path("/var/lib/limeos/integrations/jellyfin.json").write_text('{"api_key": "synthetic-not-real"}\n')
    for name in ("actions", "automation", "findings", "supervision"):
        with sqlite3.connect(f"/var/lib/limeos/agent-actions/{name}.sqlite") as db:
            db.execute("create table t (a)")
            db.execute("insert into t values (1)")
    sh(f"python3 {app}/collector.py")  # creates metrics.sqlite with the first sample
    # Units.
    unit_file("pi-health.service", f"[Unit]\nDescription=Pi-Health (frozen app imitation)\nAfter=network.target\n[Service]\nUser=holly\nWorkingDirectory={app}\nExecStart=/usr/bin/python3 {app}/app.py\nRestart=on-failure\n[Install]\nWantedBy=multi-user.target\n")
    unit_file("pihealth-helper.service", "[Unit]\nDescription=Pi-Health helper (root)\nPartOf=pi-health.service\n[Service]\nExecStart=/bin/sleep infinity\n[Install]\nWantedBy=multi-user.target\n")
    unit_file("limeos-metrics-collector.service", f"[Unit]\nDescription=metrics collector\n[Service]\nType=oneshot\nExecStart=/usr/bin/python3 {app}/collector.py\n")
    unit_file("limeos-metrics-collector.timer", "[Unit]\nDescription=metrics collector timer\n[Timer]\nOnBootSec=5s\nOnUnitActiveSec=20s\n[Install]\nWantedBy=timers.target\n")
    unit_file("limeos-package-reconcile.service", "[Unit]\nDescription=package reconcile\n[Service]\nType=oneshot\nExecStart=/bin/true\n")
    unit_file("limeos-package-reconcile.timer", "[Unit]\nDescription=package reconcile timer\n[Timer]\nOnCalendar=hourly\nPersistent=false\n[Install]\nWantedBy=timers.target\n")
    unit_file("limeops-actuatord.service", "[Unit]\nDescription=limeops actuatord\nAfter=pihealth-helper.service\n[Service]\nExecStart=/bin/sleep infinity\n[Install]\nWantedBy=multi-user.target\n")
    for name in ("limeops-worker", "limeos-agent", "flask-shutdown"):
        unit_file(f"{name}.service", f"[Unit]\nDescription={name}\n[Service]\nExecStart=/bin/sleep infinity\n[Install]\nWantedBy=multi-user.target\n")
    sh("systemctl daemon-reload")
    sh("systemctl enable --now pi-health.service pihealth-helper.service limeos-metrics-collector.timer limeos-package-reconcile.timer limeops-actuatord.service limeops-worker.service limeos-agent.service flask-shutdown.service", log="enable-fixture")
    fx.eventually(lambda: plain_get(8002)[0] == 200, timeout=20)
    # Docker: image from static busybox; containers created by holly in the docker group.
    fx.eventually(lambda: sh("docker info", check_rc=False)[0] == 0, timeout=90)
    rootfs = Path("/root/cutover-rootfs.tar")
    with tarfile.open(rootfs, "w") as archive:
        archive.add("/bin/busybox", arcname="bin/busybox")
    sh(f"docker import {rootfs} {IMAGE}")
    stack = Path("/opt/stacks/media")
    stack.mkdir(parents=True)
    (stack / "compose.yaml").write_text("services:\n  jellyfin:\n    image: x\n  radarr:\n    image: x\n  sonarr:\n    image: x\n  transmission:\n    image: x\n  airsonic-advanced:\n    image: x\n")
    sh("chown -R holly:holly /opt/stacks; chmod 0755 /opt/stacks /opt/stacks/media; chmod 0644 /opt/stacks/media/compose.yaml")

    def create(name, project=None, extra=""):
        labels = ""
        if project:
            labels = (
                f"--label com.docker.compose.project={project} --label com.docker.compose.service={name} "
                f"--label com.docker.compose.project.working_dir=/opt/stacks/{project} "
                f"--label com.docker.compose.project.config_files=/opt/stacks/{project}/compose.yaml --label com.docker.compose.oneoff=False "
            )
        command = f"echo {name} ready; trap 'exit 0' TERM INT; while :; do /bin/busybox sleep 1; done"
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
    state["ids"] = ids
    # The management catalog (the stack reader's registration); no tool writes it, so the operator step is scripted here.
    engine_id = sh("docker info --format '{{.ID}}'")[1].strip()
    catalog = {
        "version": 1,
        "engine_id": engine_id,
        "stacks": [
            {
                "stack": "media",
                "project": "media",
                "working_directory": "/opt/stacks/media",
                "operator": {"name": "compose.yaml", "sha256": sha(stack / "compose.yaml")},
                "managed": {"state": "absent"},
                "services": ["jellyfin", "radarr", "sonarr", "transmission", "airsonic-advanced"],
            }
        ],
    }
    CATALOG.write_text(json.dumps(catalog))
    sh(f"chown root:root {CATALOG}; chmod 0644 {CATALOG}")
    # Baseline of the frozen app's files (taken before any LimeOS package exists).
    files = []
    for root in ("/home/holly/pi-health", "/etc/limeos", "/var/lib/limeos", "/etc/caddy/Caddyfile"):
        if Path(root).is_file():
            files.append(root)
        for base, _, names in os.walk(root):
            files += [os.path.join(base, n) for n in names if not n.endswith((".sqlite", ".pyc"))]
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
    debs = glob.glob(str(repository / f"**/limeos_{version}_*.deb"), recursive=True)
    check(len(debs) == 1, "expected exactly one limeos .deb in the repository", debs=debs)
    state["deb"] = debs[0]
    state["version"] = version
    LOGS.mkdir(exist_ok=True)


# ---------------------------------------------------------------- cases
@case("0 fixture: frozen app serving 8002 as holly, bystanders, 6 running + 1 stopped container, Caddy installed but inactive")
def fixture_check():
    status, body = plain_get(8002)
    check(status == 200 and "pi-health-imitation" in body, "frozen app not serving", status=status)
    owner = sh("ps -o user= -p $(systemctl show pi-health -p MainPID --value)")[1].strip()
    check(owner == "holly", "frozen app not running as holly", owner=owner)
    check("0.0.0.0:8002" in listeners(), "8002 not listening on 0.0.0.0")
    check(all(frozen_world()[u] == {"active": "active", "enabled": "enabled"} for u in FROZEN_UNITS), "frozen units not enabled+active", world=frozen_world())
    c = containers()
    check(sorted(n for n, v in c.items() if v["running"]) == sorted(RUNNING), "running container set differs", names=list(c))
    check(not c["airsonic-advanced"]["running"], "stopped container is running")
    check("docker" in sh("id -nG holly")[1].split(), "holly not in the docker group")
    check(sh("runuser -u holly -- docker ps -q")[1].split(), "holly cannot use docker")
    check(permitted("/opt/stacks/media") == "holly:holly 755", "stacks root not owned by holly")
    check(active("caddy") == "inactive" and enabled("caddy") == "disabled", "caddy should be installed but inactive/disabled", caddy=(active("caddy"), enabled("caddy")))
    check(" 8003 " not in listeners() and ":80 " not in listeners(), "unexpected listeners")
    state["docker_info"] = sh("docker version --format '{{.Server.Version}}'")[1].strip()
    state["caddy_version"] = sh("caddy version")[1].strip()
    peer = sh("python3 - <<'E'\nimport socket,struct\ns=socket.socket(socket.AF_UNIX);s.connect('/var/run/docker.sock')\nprint(struct.unpack('3i', s.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, 12)))\nE")[1].strip()
    state["docker_socket_peer_pid_uid_gid"] = peer
    return {"docker": state["docker_info"], "caddy": state["caddy_version"], "docker_socket_peer(pid,uid,gid)": peer, "containers": {n: {"running": v["running"], "id": v["id"][:12]} for n, v in c.items()}, "bystanders": state["baseline_bystanders"]}


@case("1 package install (apt, pinned version) with frozen app untouched")
def install_package():
    before_c = containers()
    before_b = bystanders()
    sh(f"DEBIAN_FRONTEND=noninteractive apt-get install -y limeos={state['version']}", timeout=600, log="apt-install-limeos")
    fx.eventually(lambda: active("limeos-core") == "active", timeout=60)
    check(plain_get(8002)[0] == 200, "frozen app disturbed by package install")
    check(containers() == before_c, "containers changed by package install")
    check(bystanders() == before_b, "bystanders changed by package install")
    check(hash_tree() == state["frozen_hashes"], "frozen files changed by package install")
    # The operator sets the origin (runbook decision 3) and the core reloads.
    config = json.loads(Path("/etc/limeos/core.json").read_text())
    config["origin"] = SITE
    Path("/etc/limeos/core.json").write_text(json.dumps(config, indent=2) + "\n")
    sh("systemctl restart limeos-core")
    fx.eventually(lambda: sh("/usr/lib/limeos/limeosctl --socket /run/limeos-core/core.sock status", check_rc=False)[0] == 0, timeout=30)
    return {"version": state["version"]}


@case("2 migrate scan: names every state source, redacted, originals unchanged")
def scan():
    before = hash_tree()
    rc, out = sh("/usr/lib/limeos/limeosctl migrate scan --checkout /home/holly/pi-health", check_rc=False, log="migrate-scan")
    rc2, js = sh("/usr/lib/limeos/limeosctl migrate scan --checkout /home/holly/pi-health --json", check_rc=False, log="migrate-scan-json")
    check(rc == 0 and rc2 == 0, "scan failed", rc=rc, rc2=rc2, out=out[-1500:])
    for needle in ("credentials.env", "agent-policy.json", "auto_update.json", "backup.json", "package-approvals.json", "pending-actions.json", "jellyfin.json", "actions.sqlite", "automation.sqlite", "findings.sqlite", "supervision.sqlite", "metrics.sqlite", "policy-backups", "compose.yaml", "backup_config.json", "media_paths.json", "storage_plugins"):
        check(needle in out, f"scan report does not name {needle}", out=out[:1500])
    credentials = Path("/etc/limeos/credentials.env").read_text()
    for secret in re.findall(r"(?:scrypt|pbkdf2)[:\w]*\$[^\s,]+", credentials) + ["synthetic-not-real", PASSWORD]:
        check(secret not in out and secret not in js, "scan report leaks a secret")
    check(hash_tree() == before, "scan changed an original")
    classes = {k: len(re.findall(rf"\b{k}\b", out)) for k in ("imported", "deferred", "unsupported")}
    return {"classes_mentioned": classes, "report_lines": len(out.splitlines()), "report_head": out.splitlines()[:12]}


@case("3 import-users: Werkzeug scrypt and PBKDF2 hashes imported; login works; hash upgrades; idempotent; bad entry imports nothing")
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


@case("4 propose-ceiling: deny-by-default, accepted by check-ceiling; operator enable step; ceiling installed")
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
    check(value["managed_stacks"] == ["media"], "managed_stacks differ", stacks=value["managed_stacks"])
    check(sh(f"/usr/lib/limeos/limeosctl check-ceiling {proposed} {uid}", check_rc=False)[0] == 0, "check-ceiling refused the proposal")
    # Nothing acts before the flags are set: install it as proposed first and observe refusal.
    shutil_copy(proposed, POLICY)
    sh("systemctl reset-failed limeos-containerd; systemctl restart limeos-containerd")
    fx.eventually(lambda: active("limeos-containerd") == "active", timeout=30)
    time.sleep(3)  # allow_health is false in the proposal: the executor's health probe is refused by design
    status, headers, view = login("holly", via_https=False)
    check(status == 200, "login on the core failed", status=status)
    cookie, csrf = headers["set-cookie"].split(";")[0], view["csrf_token"]
    status, _, refused = fx.http("/api/v1/container/plans", "POST", {"resource": "container:" + state["ids"]["radarr"], "operation": "restart"}, cookie, csrf, origin=SITE)
    check(status != 201, "a deny-by-default ceiling allowed a restart plan", status=status, body=refused)
    denied_status = status
    # Operator-enable step (scripted): flip exactly the intended flags.
    for key in ("allow_health", "allow_container_read", "allow_container_logs", "allow_restart", "allow_start", "allow_stop", "allow_stack"):
        value[key] = True
    enabled_file = Path("/root/container-enabled.json")
    enabled_file.write_text(json.dumps(value, indent=2) + "\n")
    check(sh(f"/usr/lib/limeos/limeosctl check-ceiling {enabled_file} {uid}", check_rc=False)[0] == 0, "check-ceiling refused the enabled ceiling")
    shutil_copy(enabled_file, POLICY)
    sh("systemctl reset-failed limeos-containerd; systemctl restart limeos-containerd")
    fx.wait_container_ready(timeout=45)
    state["policy"] = value
    return {"denied_plan_status": denied_status, "proposal_listing": out.splitlines()[:12], "managed": len(value["managed_containers"]), "stacks": value["managed_stacks"], "enabled": [k for k, v in value.items() if k.startswith("allow_") and v]}


def shutil_copy(source, destination):
    temporary = f"{destination}.new"
    sh(f"install -m 0644 -o root -g root {source} {temporary} && mv -f {temporary} {destination}")


def snapshot_ready(name):
    """Fresh preflight; the managed file lists the six running containers."""
    managed = Path("/root/managed.txt")
    managed.write_text("\n".join(RUNNING) + "\n")
    out_dir = f"/var/lib/limeos-cutover/{name}"
    rc, out = run_script("preflight.sh", "--out", out_dir, "--managed-file", managed, log=f"preflight-{name}")
    return rc, out, Path(out_dir)


def deb_args(version=None, host=HOST, port=PORT, extra=()):
    return ["--package", state["deb"], "--expected-version", version or state["version"], "--host", host, "--port", str(port), *extra]


def snap_summary(directory):
    return {n: (directory / n).read_text()[:600] for n in ("units.txt", "docker-ps.txt", "proxy.txt", "findings.txt", "meta.env", "pagesize.txt", "packages.txt")}


def expect_frozen_serving(label, rust_running=False):
    """Frozen app serves plain HTTP on 8002 as holly, nothing of the new stack answers, caddy inactive."""
    status, body = plain_get(8002)
    check(status == 200 and "pi-health-imitation" in body, f"{label}: frozen app not serving over plain HTTP", status=status, body=body[:200])
    check(active("caddy") == "inactive" and enabled("caddy") == "disabled", f"{label}: caddy not inactive/disabled")
    check(not Path("/etc/caddy/conf.d/limeos-cutover.caddy").exists(), f"{label}: Caddy site left behind")
    for u in RUST_UNITS:
        if rust_running:
            check(active(u) == "active" and enabled(u) == "enabled", f"{label}: {u} not active+enabled", state=(active(u), enabled(u)))
        else:
            check(active(u) in ("inactive", "failed") and enabled(u) == "disabled", f"{label}: {u} not stopped+disabled", state=(active(u), enabled(u)))
    check(frozen_world() == {u: {"active": "active", "enabled": "enabled"} for u in FROZEN_UNITS}, f"{label}: frozen units not restored", world=frozen_world())
    ports = listeners()
    if not rust_running:
        check(active("limeos-storage-reader") != "active", f"{label}: stack reader still running")
    check(("127.0.0.1:8003" in ports) == rust_running, f"{label}: core listening state", ports=ports)
    owner = sh("ps -o user=,comm= -p $(systemctl show pi-health -p MainPID --value)")[1].split()
    check(owner[0] == "holly", f"{label}: frozen app owner", owner=owner)
    check("caddy" not in ports, f"{label}: caddy still listening", ports=ports)


def expect_world_equal(label, before_containers, before_bystanders, strict=True):
    check(containers() == before_containers, f"{label}: container IDs/StartedAt/pids differ", before=before_containers, after=containers())
    check(bystanders() == before_bystanders, f"{label}: a bystander unit changed", before=before_bystanders, after=bystanders())
    check(hash_tree() == state["frozen_hashes"], f"{label}: a frozen-app file hash changed", changed=[p for p, h in hash_tree().items() if state["frozen_hashes"].get(p) != h])


@case("5 preflight refusals leave everything unchanged (stopped managed container; active playback)")
def preflight_refusals():
    before_c = containers()
    before_b = bystanders()
    managed = Path("/root/managed-bad.txt")
    managed.write_text("\n".join([*RUNNING, "airsonic-advanced"]) + "\n")
    rc, out = run_script("preflight.sh", "--out", "/var/lib/limeos-cutover/refused-stopped", "--managed-file", managed, log="preflight-refused-stopped")
    check(rc != 0 and "managed container not running: airsonic-advanced" in out, "stopped managed container did not refuse", out=out)
    # Active playback: a tiny local server answers /Sessions with a NowPlayingItem.

    class Sessions(BaseHTTPRequestHandler):
        def do_GET(self):
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
        managed.write_text("\n".join(RUNNING) + "\n")
        rc2, out2 = run_script("preflight.sh", "--out", "/var/lib/limeos-cutover/refused-playback", "--managed-file", managed, "--jellyfin-url", "http://127.0.0.1:18096/Sessions", log="preflight-refused-playback")
    finally:
        server.shutdown()
    check(rc2 != 0 and "Jellyfin has active playback" in out2, "active playback did not refuse", out=out2)
    # A refused snapshot cannot be activated.
    rc3, out3 = run_script("activate.sh", "/var/lib/limeos-cutover/refused-playback", "--yes-i-have-operator-approval", *deb_args(), log="activate-refused-snapshot")
    check(rc3 != 0 and "refused preflight" in out3, "activate accepted a refused snapshot", out=out3)
    expect_world_equal("after refusals", before_c, before_b)
    expect_frozen_serving("after refusals", rust_running=True)
    return {"stopped": out.strip().splitlines()[-2:], "playback": out2.strip().splitlines()[-2:], "activate_on_refused": out3.strip().splitlines()[-1:]}


@case("6 cycle 1: preflight, dry-run (no change), activate; invariants; HTTPS login, container list/status/logs")
def cycle1_activate():
    before_c = containers()
    before_b = bystanders()
    state["c1_containers"] = before_c
    rc, out, snap = snapshot_ready("snap1")
    check(rc == 0 and "preflight passed" in out, "preflight did not pass", out=out)
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
    # Let the collector write once more so the SQLite file really changes between preflight and the stop.
    time.sleep(22)
    started = time.monotonic()
    rc, out = run_script("activate.sh", snap, "--yes-i-have-operator-approval", *deb_args(), log="activate-1")
    elapsed = round(time.monotonic() - started, 1)
    check(rc == 0 and "activate complete" in out, "activate failed", out=out[-3000:])
    # Activation itself changed no container, no bystander, no frozen-app file.
    expect_world_equal("after activation", before_c, before_b)
    check(frozen_world() == {u: {"active": "inactive", "enabled": "disabled"} for u in FROZEN_UNITS}, "frozen units not stopped+disabled", world=frozen_world())
    check(all(active(u) == "active" and enabled(u) == "enabled" for u in RUST_UNITS), "Rust services not active+enabled")
    check(active("caddy") == "active" and enabled("caddy") == "enabled", "caddy not active+enabled")
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
    return {"activate_seconds": elapsed, "activate_output_tail": out.strip().splitlines()[-12:], "poststop_baseline": state["poststop_baseline_exists"], "listeners": ports}


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
    status, _, stacks = https("/api/v1/deployment/stacks", cookie=cookie)
    host = https("/api/v1/resources", cookie=cookie)
    return {"overview_kinds": kinds, "overview_keys": sorted(overview), "containers_listed": len(listed), "log_first_line": logs["text"].splitlines()[:2], "unmanaged_logs": unmanaged, "stacks_status": status, "stacks": str(stacks)[:900], "resources_status": host[0], "resources": str(host[2])[:600]}


@case("8 cycle 1: return.sh restores the frozen app; containers/hashes/bystanders identical to the preflight snapshot; idempotent")
def cycle1_return():
    before_c = state["c1_containers"]
    rc, out = run_script("return.sh", state["snap1"], log="return-1")
    check(rc == 0 and "return verified" in out, "return.sh failed", out=out[-3000:])
    expect_frozen_serving("after return")
    expect_world_equal("after return", before_c, state["baseline_bystanders"])
    s1 = sqlite_facts()
    rc2, out2 = run_script("return.sh", state["snap1"], log="return-1-again")
    check(rc2 == 0 and "return verified" in out2, "return.sh is not idempotent", out=out2[-3000:])
    expect_frozen_serving("after second return")
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
    # The stack inventory needs the dormant reader; record whether the cutover scripts start it.
    reader_before = active("limeos-storage-reader")
    state["notes"].append(f"limeos-storage-reader after activate.sh: {reader_before}")
    if reader_before != "active":
        sh("systemctl start limeos-storage-reader", check_rc=False)

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
    check("StartedAt differs" in out, "return.sh did not name the StartedAt difference", out=out[-1500:])
    expect_frozen_serving("after return 2")
    return {"tail": out.strip().splitlines()[-5:]}


# --- cycle 3: failed activations --------------------------------------------------------------------------------
def failed_activation(label, snap_name, extra, expect_text, version=None):
    before_c = containers()
    before_b = bystanders()
    rc, out, snap = snapshot_ready(snap_name)
    check(rc == 0, "preflight did not pass", out=out)
    rc, out = run_script("activate.sh", snap, "--yes-i-have-operator-approval", *deb_args(version=version, extra=extra), log=f"activate-{label}", timeout=600)
    check(rc == 1, "activate did not fail", rc=rc, out=out[-2500:])
    check(expect_text in out and "ROLLBACK: running return.sh" in out and "return verified" in out, "no automatic verified rollback", out=out[-3000:])
    expect_frozen_serving(f"after {label}")
    # The frozen app is serving again; containers and files are exactly as before.
    check({n: v for n, v in containers().items()} == before_c, "containers changed", before=before_c)
    check(bystanders() == before_b, "bystanders changed")
    check(hash_tree() == state["frozen_hashes"], "frozen files changed")
    rc2, out2 = run_script("return.sh", snap, log=f"return-{label}-again")
    check(rc2 == 0 and "return verified" in out2, "return.sh after rollback is not idempotent", out=out2[-2000:])
    expect_frozen_serving(f"after {label} + return")
    return {"activate_tail": out.strip().splitlines()[-14:], "return_again_tail": out2.strip().splitlines()[-3:]}


@case("13 failed activation (proxy cannot reach the core: health check fails): automatic rollback, frozen app serving, return.sh idempotent")
def failed_health():
    template = Path("/root/bad-template.caddy")
    template.write_text((SRC / "packaging/Caddyfile.cutover.example").read_text().replace("127.0.0.1:8003", "127.0.0.1:8999"))
    return failed_activation("failed-health", "snap3", ["--template", str(template)], "health check failed: core health via the proxy")


@case("14 failed activation (wrong --expected-version after the package step): automatic rollback, frozen app serving")
def failed_version():
    return failed_activation("failed-version", "snap4", [], "installed limeos version is not", version="9.9.9")


@case("15 negative: activate refuses without approval, with a stale snapshot, with the wrong origin; changes nothing")
def activation_refusals():
    before_c = containers()
    rc, out, snap = snapshot_ready("snap5")
    check(rc == 0, "preflight", out=out)
    rc1, out1 = run_script("activate.sh", snap, *deb_args(), log="activate-no-approval")
    check(rc1 != 0 and "operator-approval" in out1, "activated without approval", out=out1)
    rc2, out2 = run_script("activate.sh", snap, "--yes-i-have-operator-approval", *deb_args(host="other.lan"), log="activate-wrong-origin")
    check(rc2 != 0 and "origin must be" in out2, "wrong origin not refused", out=out2)
    Path("/root/stale-snap").mkdir(exist_ok=True)
    sh(f"rm -rf /root/stale-snap/* && cp -a {snap}/. /root/stale-snap/ && sed -i 's/^epoch=.*/epoch=1/' /root/stale-snap/meta.env")
    rc3, out3 = run_script("activate.sh", "/root/stale-snap", "--yes-i-have-operator-approval", *deb_args(), log="activate-stale")
    check(rc3 != 0 and "stale" in out3, "stale snapshot accepted", out=out3)
    expect_frozen_serving("after refusals 2")
    check(containers() == before_c, "containers changed")
    return {"no_approval": out1.strip().splitlines()[-1], "wrong_origin": out2.strip().splitlines()[-1], "stale": out3.strip().splitlines()[-1]}


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
            "binary_sha256": {n: sha(f"/usr/lib/limeos/{n}") for n in ("limeos-core", "limeos-executor", "limeosctl", "limeos-password-worker")},
            "script_sha256": {n: sha(CUTOVER / n) for n in ("lib.sh", "preflight.sh", "activate.sh", "return.sh")},
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
