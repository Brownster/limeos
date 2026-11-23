"""Stub-driven tests for scripts/cutover/{preflight,activate,return,measure}.sh.

No real systemctl, docker, ss, free, curl, dpkg or caddy is ever run: PATH points
at copies of tests/cutover/stubs/stub.sh, and host paths live under a temp root.
Run: python3 -m unittest discover -s tests/cutover
"""

import fcntl
import hashlib
import os
import re
import shutil
import signal
import subprocess
import tempfile
import time
import types
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
SCRIPTS = REPO / "scripts/cutover"
STUB = Path(__file__).resolve().parent / "stubs/stub.sh"
TOOLS = [
    "systemctl", "systemd-run", "docker", "ss", "free", "curl", "dpkg", "dpkg-deb", "dpkg-query",
    "caddy", "sleep", "limeosctl", "stat",
]
NOW = 1_800_000_000
JELLYFIN_KEY = "0123456789abcdef0123456789abcdef"
SESSIONS = "http://127.0.0.1:8096/Sessions"
HOST = "lime.lan"
SITE = f"https://{HOST}:8002"
MANAGED = [
    "jellyfin", "radarr", "sonarr", "sabnzbd", "transmission", "jackett", "vpn",
    "navidrome", "audiobookshelf", "rdtclient", "requestrr", "get_iplayer",
    "yt-to-jellyfin", "watchtower",
]
LIMEOPS = ["limeops-actuatord.service", "limeops-worker.service"]
UNTOUCHED = ["limeos-agent.service", "flask-shutdown.service", *LIMEOPS]
FROZEN = [
    "pihealth-helper.service", "pi-health.service",
    "limeos-metrics-collector.timer", "limeos-package-reconcile.timer",
]
RUST = ["limeos-storaged.service", "limeos-containerd.service", "limeos-core.service"]
READER = "limeos-storage-reader.service"
READER_SOCKET = "run/limeos-storage-reader/executor.sock"
CATALOG = "etc/limeos/management-catalog.json"
MUTATING_VERBS = {"stop", "start", "restart", "enable", "disable", "reload", "daemon-reload", "kill",
                  "reload-or-restart", "try-reload-or-restart"}
# Every mutating call of a successful activation, in order.
ACTIVATE_STEPS = [
    "systemctl stop limeos-metrics-collector.timer limeos-package-reconcile.timer",
    "systemctl stop pi-health.service",
    "systemctl stop pihealth-helper.service",
    "systemctl disable pihealth-helper.service",
    "systemctl disable pi-health.service",
    "systemctl disable limeos-metrics-collector.timer",
    "systemctl disable limeos-package-reconcile.timer",
    "dpkg --force-confold -i /pkg/limeos_1.0.0_arm64.deb",
    "systemctl daemon-reload",
    "systemctl enable limeos-storaged.service",
    "systemctl enable limeos-containerd.service",
    "systemctl enable limeos-core.service",
    "systemctl start limeos-storaged.service",
    "systemctl start limeos-containerd.service",
    "systemctl start limeos-core.service",
    f"systemctl start {READER}",
    "caddy validate --adapter caddyfile --config /etc/caddy/Caddyfile",
    "systemctl enable caddy.service",
    "systemctl start caddy.service",
]
FREE_OK = (
    "              total        used        free      shared  buff/cache   available\n"
    "Mem:     8000000000  2000000000  4000000000     1000000  2000000000  6000000000\n"
    "Swap:     535822336   420000000   115822336\n"
)


def tree_digest(*roots, skip=("calls.log", "scoped-calls.log")):
    digest = hashlib.sha256()
    for root in roots:
        for path in sorted(Path(root).rglob("*")):
            if path.name in skip or not path.is_file():
                continue
            digest.update(str(path.relative_to(root)).encode())
            digest.update(path.read_bytes())
    return digest.hexdigest()


class Harness(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory(prefix="cutover-test-")
        self.addCleanup(self._tmp.cleanup)
        base = Path(self._tmp.name)
        self.root = base / "root"
        self.stub = base / "stub"
        self.bin = base / "bin"
        for directory in (self.root, self.stub / "units", self.bin):
            directory.mkdir(parents=True)
        for tool in TOOLS:
            shutil.copy(STUB, self.bin / tool)
            (self.bin / tool).chmod(0o755)
        self.env = {
            **os.environ,
            "PATH": f"{self.bin}:{os.environ['PATH']}",
            "STUB_DIR": str(self.stub),
            "LIMEOS_CUTOVER_ROOT": str(self.root),
            "CUTOVER_ALLOW_NONROOT": "1",
            "CUTOVER_NOW": str(NOW),
            "LIMEOSCTL": str(self.bin / "limeosctl"),
        }
        # Runs after the test (cleanups are last in, first out), before the temp tree goes:
        # kills whatever the test started, detached runs and blocked stubs included.
        self.addCleanup(self.kill_strays, self.stub)
        self.out = "/var/lib/limeos-cutover/snap"
        self.snap = self.root / self.out.lstrip("/")
        # The operator's state list, as built from the `limeosctl migrate scan` report.
        self.state_list = base / "state-paths.txt"
        self.state_list.write_text(
            "# from limeosctl migrate scan\n/etc/limeos/credentials.env\n"
            "/home/holly/pi-health/config/backup_config.json\n/home/holly/pi-health/config/media_paths.json\n")
        self.key_file = base / "jellyfin.key"
        self.key_file.write_text(JELLYFIN_KEY + "\n")
        self.key_file.chmod(0o600)
        self._seed_host()

    @staticmethod
    def marked_pids(stub_dir):
        """Live processes whose environment carries this harness's STUB_DIR."""
        marker = b"\0STUB_DIR=" + os.fsencode(str(stub_dir)) + b"\0"
        found = []
        for entry in os.listdir("/proc"):
            if not entry.isdigit() or int(entry) == os.getpid():
                continue
            try:
                with open(f"/proc/{entry}/environ", "rb") as handle:
                    environ = b"\0" + handle.read()
                with open(f"/proc/{entry}/stat") as handle:
                    state = handle.read().rsplit(") ", 1)[1].split()[0]
            except OSError:
                continue
            if marker in environ and state != "Z":
                found.append(int(entry))
        return found

    @classmethod
    def kill_strays(cls, stub_dir):
        """SIGKILL every process whose environment carries this harness's STUB_DIR."""
        for pid in cls.marked_pids(stub_dir):
            try:
                os.kill(pid, signal.SIGKILL)
            except OSError:
                pass

    def _write(self, path, text):
        target = self.root / path.lstrip("/")
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(text)

    def _unit(self, name, active="active", enabled="enabled"):
        (self.stub / "units" / f"{name}.active").write_text(active + "\n")
        (self.stub / "units" / f"{name}.enabled").write_text(enabled + "\n")

    def _seed_host(self):
        self._write("/etc/limeos/credentials.env", "PIHEALTH_USERS=holly:SECRET-HASH-VALUE\n")
        self._write("/etc/limeos/core.json", f'{{"version":1,"listen":"127.0.0.1:8003","origin":"{SITE}"}}\n')
        self._write("/var/lib/limeos/metrics.db", "sqlite-bytes-1")
        self._write("/var/lib/limeos/auto_update.json", "{}")
        self._write("/home/holly/pi-health/config/backup_config.json", '{"edited":1}')
        self._write("/home/holly/pi-health/config/media_paths.json", '{"m":1}')
        self._write("/" + CATALOG, '{"version":1}\n')
        (self.root / CATALOG).chmod(0o644)
        self.reader_listens()
        self._write("/etc/caddy/Caddyfile", "import /etc/caddy/conf.d/*.caddy\n")
        self._write("/pkg/limeos_1.0.0_arm64.deb", "deb")
        # The cgroup2 unified hierarchy with cgroup.kill (kernel 5.14 or newer), as on Debian 12.
        self._write("/sys/fs/cgroup/system.slice/cgroup.kill", "")
        for name in [*FROZEN, *UNTOUCHED]:
            self._unit(name)
        self._unit("caddy.service", "inactive", "disabled")
        for name in [*RUST, READER]:
            self._unit(name, "inactive", "disabled")
        ids = [f"{i:064x}" for i in range(1, len(MANAGED) + 1)]
        (self.stub / "docker_ps.txt").write_text(
            "".join(f"{i} {n}\n" for i, n in zip(ids, MANAGED)))
        (self.stub / "docker_started.txt").write_text(
            "".join(f"{i} /{n} 2026-11-01T00:00:00Z\n" for i, n in zip(ids, MANAGED)))
        (self.stub / "ss_out").write_text(
            "LISTEN 0 128 0.0.0.0:8002 0.0.0.0:*\nLISTEN 0 128 0.0.0.0:8009 0.0.0.0:*\n")
        (self.stub / "free_out").write_text(FREE_OK)
        (self.stub / "pkg_to_install").write_text("1.0.0\n")
        self.set_curl({
            SESSIONS: "200 []",
            f"{SITE}/api/v1/health": "200",
            f"{SITE}/": "200 <html>",
            "http://127.0.0.1:8002/": "200",
        })

    def reader_listens(self, listens=True):
        """Starting the reader creates its socket (or, with listens=False, does not)."""
        script = self.stub / f"hook-start-{READER}"
        if not listens:
            script.unlink(missing_ok=True)
            return
        sock = self.root / READER_SOCKET
        script.write_text(
            "#!/usr/bin/env bash\n"
            f"mkdir -p '{sock.parent}' && rm -f '{sock}' && "
            f"python3 -c \"import socket,sys; socket.socket(socket.AF_UNIX).bind(sys.argv[1])\" '{sock}'\n")
        script.chmod(0o755)

    def set_curl(self, mapping):
        (self.stub / "curl.map").write_text("".join(f"{k} {v}\n" for k, v in mapping.items()))

    def inject(self, call):
        with open(self.stub / "fail.txt", "a") as handle:
            handle.write(call + "\n")

    def hang(self, *patterns):
        """Stub calls matching any glob PATTERN hang for 90 s (a stuck systemd query)."""
        with open(self.stub / "hang.txt", "a") as handle:
            handle.write("".join(p + "\n" for p in patterns))

    def silent(self, *patterns):
        """Stub calls matching any glob PATTERN print nothing and fail (the tool could not answer)."""
        with open(self.stub / "silent.txt", "a") as handle:
            handle.write("".join(p + "\n" for p in patterns))

    def run_script(self, name, *args, now=NOW, new_session=False, timeout=60):
        env = {**self.env, "CUTOVER_NOW": str(now)}
        return subprocess.run(
            ["bash", str(SCRIPTS / name), *args], env=env, capture_output=True, text=True, timeout=timeout,
            start_new_session=new_session)

    def preflight(self, *extra, state_list=True, jellyfin_key=True):
        listed = ["--state-list", str(self.state_list)] if state_list else []
        if jellyfin_key:
            listed += ["--jellyfin-key-file", str(self.key_file)]
        return self.run_script("preflight.sh", "--out", self.out, *listed, *extra)

    def activate_args(self, *extra):
        return [
            str(Path(self.out)), "--package", "/pkg/limeos_1.0.0_arm64.deb",
            "--expected-version", "1.0.0", "--host", HOST, *extra,
        ]

    def activate(self, *extra, now=NOW + 60, attached=True, timeout=60):
        args = self.activate_args(*extra, *(["--attached"] if attached else []))
        args[0] = str(self.snap)
        return self.run_script("activate.sh", *args, now=now, timeout=timeout)

    def approved(self, *extra, now=NOW + 60, attached=True, timeout=60):
        return self.activate("--yes-i-have-operator-approval", *extra, now=now, attached=attached, timeout=timeout)

    def do_return(self, *extra):
        return self.run_script("return.sh", str(self.snap), *extra)

    def path_without(self, tool):
        """A PATH holding the stubs and every system command except TOOL."""
        shadow = Path(self._tmp.name) / f"without-{tool}"
        shadow.mkdir()
        (self.bin / tool).unlink(missing_ok=True)
        for directory in os.environ["PATH"].split(":"):
            if not os.path.isdir(directory):
                continue
            for name in os.listdir(directory):
                link = shadow / name
                if name != tool and not link.exists() and not link.is_symlink():
                    link.symlink_to(os.path.join(directory, name))
        return f"{self.bin}:{shadow}"

    def env_without_default_signal(self):
        real = shutil.which("env")
        self.fake_tool("env", 'if [[ "${1:-}" == --default-signal* ]]; then echo "env: unrecognized option" >&2; exit 125; fi\n'
                              f'exec "{real}" "$@"')

    @staticmethod
    def wait_for(predicate, timeout=30):
        deadline = time.monotonic() + timeout
        while not predicate():
            if time.monotonic() > deadline:
                raise AssertionError("timed out")
            time.sleep(0.05)

    def fake_tool(self, name, script="exit 1"):
        """Shadow a real tool (sha256sum, mktemp, ...) with a script in the stub PATH."""
        tool = self.bin / name
        tool.write_text(f"#!/usr/bin/env bash\n{script}\n")
        tool.chmod(0o755)

    def calls(self):
        path = self.stub / "calls.log"
        return path.read_text().splitlines() if path.exists() else []

    def mutating_calls(self):
        found = []
        for line in self.calls():
            tool, _, rest = line.partition(" ")
            verb = rest.split(" ")[0]
            if tool == "systemctl" and verb in MUTATING_VERBS:
                found.append(line)
            elif tool == "caddy" and "--config /tmp/" in line:
                continue  # pre-change validation of the rendered site is read-only
            elif tool in ("dpkg", "caddy") or (tool == "docker" and verb not in ("ps", "inspect")):
                found.append(line)
        return found

    def unit_state(self, name):
        def read(kind):
            return (self.stub / "units" / f"{name}.{kind}").read_text().strip()
        return read("active"), read("enabled")

    def passing_snapshot(self):
        result = self.preflight()
        self.assertEqual(result.returncode, 0, result.stderr)
        (self.stub / "calls.log").write_text("")

    def scope_unit(self, snap=None):
        """The transient scope the scripts name for a snapshot (this one by default)."""
        unit = subprocess.run(
            ["bash", "-c", f'source "{SCRIPTS}/lib.sh"; fct_scope_unit "$1"', "_", str(snap or self.snap)],
            capture_output=True, text=True, check=True).stdout.strip()
        self.assertRegex(unit, r"^limeos-cutover-activate-[0-9a-f]{32}\.scope$")
        return unit

    def hold_lock_with_record(self, record, name="activate.lock"):
        """Hold a snapshot lock from the test process, with RECORD as the holder line."""
        lock = self.snap / name
        handle = open(lock, "a")
        self.addCleanup(handle.close)
        fcntl.flock(handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
        lock.write_text(record)
        return handle

    def own_record(self, role, unit=""):
        """A current holder record naming the test process."""
        start = Path(f"/proc/{os.getpid()}/stat").read_text().rsplit(") ", 1)[1].split()[19]
        return f"{role} {os.getpid()} {start}{' ' + unit if unit else ''}\n"

    def scope_member(self, unit):
        """A process inside the stub scope UNIT (cleaned up with the harness's other processes)."""
        member = subprocess.Popen(["bash", "-c", "while :; do command -p sleep 0.05; done"],
                                  env={**self.env, "STUB_SCOPE": unit}, start_new_session=True)
        self.addCleanup(lambda: (member.kill(), member.wait()))
        return member

    def scoped_calls(self):
        """(scope, call) for every stub call; scope is "-" outside any transient scope."""
        path = self.stub / "scoped-calls.log"
        lines = path.read_text().splitlines() if path.exists() else []
        return [tuple(line.split(" ", 1)) for line in lines]

    def pids_with(self, marker):
        """Live harness processes whose environment carries MARKER (bytes, e.g. b"STUB_UNKILLABLE=1")."""
        found = []
        for pid in self.marked_pids(self.stub):
            try:
                with open(f"/proc/{pid}/environ", "rb") as handle:
                    if marker in handle.read().split(b"\0"):
                        found.append(pid)
            except OSError:
                pass
        return found

    def lock_free(self, name):
        with open(self.snap / name, "a") as handle:
            try:
                fcntl.flock(handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except BlockingIOError:
                return False
            fcntl.flock(handle, fcntl.LOCK_UN)
            return True

    @staticmethod
    def alive(pid):
        try:
            with open(f"/proc/{pid}/stat") as handle:
                return handle.read().rsplit(") ", 1)[1].split()[0] != "Z"
        except OSError:
            return False


class HarnessTests(Harness):
    def test_cleanup_kills_processes_left_in_other_sessions(self):
        # A blocked stub in its own session survives its script's death; the cleanup must not leak it.
        child = subprocess.Popen(["bash", "-c", "while :; do command -p sleep 0.05; done"],
                                 env=self.env, start_new_session=True)
        self.kill_strays(self.stub)
        self.assertEqual(child.wait(timeout=10), -signal.SIGKILL)


class PreflightTests(Harness):
    def test_snapshot_written_and_nothing_else_changes(self):
        before = tree_digest(self.root), tree_digest(self.stub / "units")
        result = self.preflight()
        self.assertEqual(result.returncode, 0, result.stderr)
        for name in ("meta.env", "units.txt", "ports.txt", "docker-ps.txt", "docker-started.txt",
                     "state.sha256", "memory.txt", "pagesize.txt", "proxy.txt", "packages.txt"):
            self.assertTrue((self.snap / name).is_file(), name)
        self.assertEqual(self.mutating_calls(), [])
        # Only the snapshot directory is new.
        shutil.rmtree(self.root / "var/lib/limeos-cutover")
        self.assertEqual(before, (tree_digest(self.root), tree_digest(self.stub / "units")))

    def test_snapshot_never_holds_file_contents(self):
        self.preflight()
        for path in self.snap.iterdir():
            self.assertNotIn("SECRET-HASH-VALUE", path.read_text())
        self.assertIn("/etc/limeos/credentials.env", (self.snap / "state.sha256").read_text())

    def test_records_untouched_units_and_caddy(self):
        self.preflight()
        units = (self.snap / "units.txt").read_text()
        for name in [*UNTOUCHED, "caddy.service", *FROZEN]:
            self.assertIn(name, units)

    def refused(self, fragment):
        result = self.preflight()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(fragment, result.stderr)
        self.assertEqual(self.mutating_calls(), [])

    def test_refuses_low_memory(self):
        (self.stub / "free_out").write_text(FREE_OK.replace("6000000000", "1000000000"))
        self.refused("below 1.5 GiB")

    def test_refuses_high_swap(self):
        (self.stub / "free_out").write_text(FREE_OK.replace("420000000", "510000000"))
        self.refused("swap use is above 90%")

    def test_refuses_a_missing_catalog_with_the_exact_command(self):
        (self.root / CATALOG).unlink()
        result = self.preflight()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("REFUSED: stack management catalog: /etc/limeos/management-catalog.json is missing", result.stderr)
        self.assertIn(f"{self.bin}/limeosctl migrate propose-catalog --out /root/management-catalog.proposed.json --install", result.stderr)
        self.assertIn("install -o root -g root -m 0644", result.stderr)
        self.assertEqual(self.mutating_calls(), [])

    def test_refuses_a_catalog_that_is_a_symlink_or_writable_by_others(self):
        path = self.root / CATALOG
        path.chmod(0o666)
        self.refused("not group/world writable")
        path.unlink()
        path.symlink_to("/etc/hostname")
        shutil.rmtree(self.root / "var/lib/limeos-cutover")
        self.refused("not a regular file")

    def test_the_last_line_of_operator_lists_counts_without_a_newline(self):
        managed = Path(self._tmp.name) / "managed.txt"
        managed.write_text("jellyfin\nradarr")
        self.state_list.write_text("/etc/limeos/credentials.env\n/home/holly/pi-health/config/media_paths.json")
        lines = (self.stub / "docker_ps.txt").read_text().splitlines()
        (self.stub / "docker_ps.txt").write_text("\n".join(l for l in lines if "radarr" not in l) + "\n")
        result = self.preflight("--managed-file", str(managed))
        self.assertIn("managed container not running: radarr", result.stderr)
        self.assertIn("/home/holly/pi-health/config/media_paths.json\n", (self.snap / "state-paths.txt").read_text())
        self.assertEqual((self.snap / "managed.txt").read_text(), "jellyfin\nradarr\n")

    def test_refuses_stopped_managed_container(self):
        lines = (self.stub / "docker_ps.txt").read_text().splitlines()
        (self.stub / "docker_ps.txt").write_text("\n".join(l for l in lines if "radarr" not in l) + "\n")
        self.refused("managed container not running: radarr")

    def test_refuses_active_playback(self):
        self.set_curl({SESSIONS: '200 [{"NowPlayingItem":{}}]'})
        self.refused("active playback")

    def test_idle_jellyfin_passes_and_the_key_stays_out_of_argv_and_the_snapshot(self):
        result = self.preflight()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn(f"curl -sS --max-time 5 -H @- -o {self.snap}/.jellyfin-body -w %{{http_code}} {SESSIONS}",
                      self.calls())
        # The key reaches curl on stdin, never on a command line or in the snapshot.
        self.assertEqual((self.stub / "curl_headers").read_text(), f'Authorization: MediaBrowser Token="{JELLYFIN_KEY}"\n')
        self.assertNotIn(JELLYFIN_KEY, (self.stub / "calls.log").read_text())
        for path in self.snap.iterdir():
            self.assertNotIn(JELLYFIN_KEY, path.read_text())

    def test_refuses_without_an_api_key(self):
        result = self.preflight(jellyfin_key=False)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("REFUSED: no Jellyfin API key", result.stderr)
        self.assertFalse([l for l in self.calls() if SESSIONS in l])

    def test_no_playback_check_skips_jellyfin_with_a_warning(self):
        result = self.preflight("--no-playback-check", jellyfin_key=False)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("--no-playback-check", result.stderr)
        self.assertIn("check the dashboard by hand", (self.snap / "findings.txt").read_text())
        self.assertFalse([l for l in self.calls() if SESSIONS in l])

    def test_refuses_a_key_file_others_can_read(self):
        for mode in (0o640, 0o604, 0o660):
            with self.subTest(mode=oct(mode)):
                self.setUp()
                self.key_file.chmod(mode)
                self.refused("readable by root only")
                self.assertFalse([l for l in self.calls() if SESSIONS in l])

    def test_refuses_a_rejected_key_or_an_unreachable_jellyfin(self):
        for code, fragment in (("401", "Jellyfin rejected the API key (HTTP 401)"),
                               (None, "Jellyfin playback could not be checked (HTTP 000, curl exit 7)")):
            with self.subTest(code=code):
                self.setUp()
                self.set_curl({SESSIONS: code} if code else {})
                self.refused(fragment)

    def test_a_200_that_is_cut_short_or_not_a_session_list_is_refused(self):
        cases = (
            ('200 []', "28", "Jellyfin playback could not be checked (HTTP 200, curl exit 28)"),
            ('200 [{"Id":"a"', None, "Jellyfin /Sessions did not return a JSON list of sessions"),
            ('200 <html>login</html>', None, "Jellyfin /Sessions did not return a JSON list of sessions"),
            ('200 {"Items":[]}', None, "Jellyfin /Sessions did not return a JSON list of sessions"),
        )
        for answer, curl_exit, fragment in cases:
            with self.subTest(answer=answer, curl_exit=curl_exit):
                self.setUp()
                self.set_curl({SESSIONS: answer})
                if curl_exit:
                    (self.stub / "curl_exit").write_text(curl_exit)
                self.refused(fragment)

    def test_refuses_a_symlinked_key_file(self):
        target = Path(self._tmp.name) / "real.key"
        self.key_file.rename(target)
        self.key_file.symlink_to(target)
        self.refused("is not a regular file")
        self.assertFalse([l for l in self.calls() if SESSIONS in l])

    def test_each_missing_requirement_is_refused_by_name(self):
        cases = (
            ("python3", "requirement: python3 is not installed (needed to read Jellyfin sessions; or pass --no-playback-check)"),
            ("systemd-run", "requirement: systemd-run is not installed"),
            ("flock", "requirement: flock is not installed"),
            ("setsid", "requirement: setsid is not installed"),
            ("env", "requirement: env does not support --default-signal (coreutils 8.31 or newer;"),
        )
        for tool, fragment in cases:
            with self.subTest(tool=tool):
                self.setUp()
                if tool == "env":
                    self.env_without_default_signal()
                else:
                    self.env["PATH"] = self.path_without(tool)
                result = self.preflight()
                self.assertNotEqual(result.returncode, 0)
                self.assertIn(fragment, result.stderr)
                self.assertNotIn("did not return a JSON list", result.stderr)

    def test_python3_is_only_needed_for_the_playback_check(self):
        self.env["PATH"] = self.path_without("python3")
        result = self.preflight("--no-playback-check", jellyfin_key=False)
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_requires_a_state_list(self):
        result = self.preflight(state_list=False)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("a state list is required: --state-list FILE", result.stderr)
        self.assertFalse(self.snap.exists())

    def test_rejects_a_relative_or_empty_state_list(self):
        for text, fragment in (("etc/limeos/credentials.env\n", "not an absolute path: etc/limeos/credentials.env"),
                               ("# nothing\n\n", "names no paths")):
            with self.subTest(text=text):
                self.state_list.write_text(text)
                result = self.preflight()
                self.assertNotEqual(result.returncode, 0)
                self.assertIn(fragment, result.stderr)
                self.assertFalse(self.snap.exists())

    def test_empty_root_is_refused_with_a_named_reason(self):
        shutil.rmtree(self.root)
        self.root.mkdir()
        result = self.preflight()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("REFUSED: state path missing: /etc/limeos/credentials.env", result.stderr)
        self.assertIn("REFUSED: no frozen-app state found", result.stderr)
        self.assertIn("preflight=refused", (self.snap / "meta.env").read_text())

    def test_a_listed_path_that_is_missing_is_refused(self):
        (self.root / "home/holly/pi-health/config/media_paths.json").unlink()
        self.refused("state path missing: /home/holly/pi-health/config/media_paths.json")

    def test_state_paths_come_from_the_list_not_a_home_directory(self):
        self._write("/srv/app/config/settings.json", '{"a":1}')
        self.state_list.write_text("/srv/app/config/settings.json\n")
        result = self.preflight()
        self.assertEqual(result.returncode, 0, result.stderr)
        hashes = (self.snap / "state.sha256").read_text()
        self.assertIn("/srv/app/config/settings.json", hashes)
        self.assertNotIn("/home/holly", hashes)
        self.assertEqual((self.snap / "state-paths.txt").read_text(), "/srv/app/config/settings.json\n")
        # activate and return hash the snapshot's list, so a change there is caught on return.
        (self.stub / "calls.log").write_text("")
        self.assertEqual(self.approved().returncode, 0)
        self._write("/srv/app/config/settings.json", '{"a":2}')
        result = self.do_return()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("state file changed: /srv/app/config/settings.json", result.stderr)

    def test_a_listed_directory_or_symlink_is_refused_not_hashed_as_present(self):
        (self.root / "srv").mkdir()
        (self.root / "srv/link.json").symlink_to(self.root / "etc/limeos/credentials.env")
        for entry in ("/etc/limeos", "/srv/link.json"):
            with self.subTest(entry=entry):
                shutil.rmtree(self.root / "var/lib/limeos-cutover", ignore_errors=True)
                self.state_list.write_text(f"/etc/limeos/credentials.env\n{entry}\n")
                result = self.preflight()
                self.assertNotEqual(result.returncode, 0)
                self.assertIn(f"state path is not a regular file: {entry}", result.stderr)
                self.assertIn("REFUSED: state files could not all be hashed", result.stderr)

    def test_a_hash_failure_is_refused(self):
        self.fake_tool("sha256sum")
        result = self.preflight()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("cannot hash /etc/limeos/credentials.env", result.stderr)
        self.assertIn("REFUSED: state files could not all be hashed", result.stderr)
        self.assertNotIn("no frozen-app state found", result.stderr)

    def test_paths_with_spaces_and_tabs_round_trip_through_activate_and_return(self):
        odd = ["/srv/my app/settings.json", "/srv/tab\there.json", "/srv/trailing space "]
        for path in odd:
            self._write(path, "x")
        self.state_list.write_text("".join(f"{p}\n" for p in odd))
        self._write("/var/lib/limeos/backup  two spaces.json", "{}")
        self.assertEqual(self.preflight().returncode, 0)
        (self.stub / "calls.log").write_text("")
        result = self.approved()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        result = self.do_return()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertNotIn("changed", result.stderr)
        # A changed odd path is still caught by name.
        self._write("/srv/trailing space ", "y")
        result = self.do_return()
        self.assertIn("state file changed: /srv/trailing space ", result.stderr)

    @unittest.skipUnless(
        "en_gb.utf8" in subprocess.run(["locale", "-a"], capture_output=True, text=True).stdout.lower(),
        "the en_GB.UTF-8 locale is not installed")
    def test_records_survive_a_locale_that_collates_distinct_names_as_equal(self):
        # en_GB.UTF-8 collates these two names as equal; sort -u there kept only one record.
        names = [b"/var/lib/limeos/a\x80.json", b"/var/lib/limeos/a\x81.json"]
        for name in names:
            with open(os.fsencode(str(self.root)) + name, "wb") as handle:
                handle.write(b"same")
        self.env["LC_ALL"] = "en_GB.UTF-8"
        self.assertEqual(self.preflight().returncode, 0)
        records = (self.snap / "state.sha256").read_bytes()
        for name in names:
            self.assertIn(b" " + name + b"\n", records)
        (self.stub / "calls.log").write_text("")
        self.assertEqual(self.approved().returncode, 0)
        with open(os.fsencode(str(self.root)) + names[1], "wb") as handle:
            handle.write(b"changed")
        result = subprocess.run(["bash", str(SCRIPTS / "return.sh"), str(self.snap)], env=self.env,
                                capture_output=True, timeout=60)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(b"state file changed: " + names[1], result.stderr)

    def test_a_state_file_name_with_a_newline_is_refused(self):
        self._write("/var/lib/limeos/evil\nname.json", "{}")
        result = self.preflight()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("contains a newline", result.stderr)

    def test_state_files_override_still_works(self):
        self.env["CUTOVER_STATE_FILES"] = "/etc/limeos/credentials.env"
        result = self.preflight(state_list=False)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertNotIn("/home/holly", (self.snap / "state.sha256").read_text())

    def test_warns_without_refusing_when_cgroup_kill_is_unavailable(self):
        cases = (("no cgroup.kill", lambda: (self.root / "sys/fs/cgroup/system.slice/cgroup.kill").unlink(),
                  "has no cgroup.kill (kernel before 5.14)"),
                 ("not cgroup2", lambda: (self.stub / "cgroupfs-tmpfs").touch(), "is not on a cgroup2 file system"))
        for label, breakage, reason in cases:
            with self.subTest(case=label):
                self.setUp()
                breakage()
                result = self.preflight()
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertIn("WARN: the cgroup2 unified hierarchy with cgroup.kill is not available", result.stderr)
                self.assertIn(reason, result.stderr)
                self.assertIn("return.sh --force could not end a hung activation", result.stderr)
                self.assertIn("--allow-systemctl-kill-fallback", result.stderr)
                self.assertIn("cgroup.kill is not available", (self.snap / "findings.txt").read_text())
        # On a host that has it, nothing is said.
        self.setUp()
        result = self.preflight()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertNotIn("cgroup.kill", result.stderr)

    def test_refused_snapshot_blocks_activate(self):
        (self.stub / "free_out").write_text(FREE_OK.replace("6000000000", "1000000000"))
        self.preflight()
        result = self.approved()
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.mutating_calls(), [])

    def test_a_hung_query_for_the_frozen_app_is_refused_and_never_recorded_blank(self):
        # Scenario A: preflight's query for pi-health hangs. A blank state would let activation stop the
        # app and a later return skip restarting it; instead preflight refuses, naming the query.
        self.env["CUTOVER_QUERY_TIMEOUT"] = "1"
        self.hang("systemctl is-active pi-health.service")
        result = self.preflight()
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("timed out after 1s: systemctl is-active pi-health.service", result.stderr)
        self.assertIn("REFUSED: could not determine whether pi-health.service is active", result.stderr)
        self.assertIn("preflight=refused", (self.snap / "meta.env").read_text())
        units = (self.snap / "units.txt").read_text()
        self.assertIn("managed pi-health.service unknown enabled\n", units)
        for line in units.splitlines():
            self.assertEqual(len(line.split(" ")), 4, line)
            self.assertNotIn("", line.split(" "), line)
        (self.stub / "hang.txt").unlink()
        result = self.approved()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("snapshot records a refused preflight", result.stderr)
        self.assertEqual(self.mutating_calls(), [])

    def test_every_unanswered_query_is_a_named_refusal(self):
        cases = (
            ("systemctl is-active limeos-agent.service", "could not determine whether limeos-agent.service is active"),
            ("systemctl is-enabled pi-health.service", "could not determine whether pi-health.service is enabled"),
            ("systemctl list-unit-files *", "systemd did not list its unit files"),
            ("systemctl is-active nginx.service", "could not determine whether nginx.service is active"),
            ("docker ps *", "could not list the running containers"),
            ("docker inspect *", "could not list the running containers"),
            ("dpkg-query *", "could not determine whether the limeos package is installed"),
            ("ss *", "could not list the listening TCP ports"),
        )
        for pattern, fragment in cases:
            with self.subTest(query=pattern):
                self.setUp()
                self.silent(pattern)
                result = self.preflight()
                self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
                self.assertIn(f"REFUSED: {fragment}", result.stderr)
                self.assertIn("preflight=refused", (self.snap / "meta.env").read_text())
                # A container list that could not be read is not a list of stopped containers.
                self.assertNotIn("managed container not running", result.stderr)
                self.assertEqual(self.mutating_calls(), [])

    def test_a_unit_file_that_does_not_exist_is_recorded_as_not_found(self):
        # Older systemd's is-enabled prints nothing for a unit file that does not exist (the Rust
        # units before the package is installed). That is a definite answer, not a missing one.
        for name in RUST:
            (self.stub / "units" / f"{name}.enabled").unlink()
            (self.stub / "units" / f"{name}.loadstate").write_text("not-found\n")
        result = self.preflight()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        units = (self.snap / "units.txt").read_text()
        for name in RUST:
            self.assertIn(f"managed {name} inactive not-found\n", units)
            self.assertIn(f"systemctl show -p LoadState --value {name}", self.calls())
        (self.stub / "calls.log").write_text("")
        self.assertEqual(self.approved().returncode, 0)
        result = self.do_return()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        for name in RUST:
            self.assertEqual(self.unit_state(name), ("inactive", "disabled"), name)

    def test_a_snapshot_with_a_blank_or_unknown_unit_state_is_refused_by_activate_and_return(self):
        # A snapshot from the preflight before this check could hold a blank state; neither script
        # may guess one (a guessed "not active" would leave the frozen app down after a return).
        cases = (
            ("blank", "managed pi-health.service  enabled"),
            ("unknown", "managed pi-health.service unknown enabled"),
            ("short", "managed pi-health.service active"),
            ("missing", None),
        )
        for label, record in cases:
            with self.subTest(record=label):
                self.setUp()
                self.passing_snapshot()
                path = self.snap / "units.txt"
                lines = [l for l in path.read_text().splitlines() if " pi-health.service " not in l]
                if record:
                    lines.append(record)
                path.write_text("\n".join(sorted(lines)) + "\n")
                for script, result in (("activate", self.approved()), ("return", self.do_return())):
                    self.assertEqual(result.returncode, 1, script + result.stdout + result.stderr)
                    self.assertIn("units.txt", result.stderr, script)
                    self.assertIn("pi-health.service", result.stderr, script)
                    self.assertIn("nothing was changed", result.stderr, script)
                    self.assertEqual(self.mutating_calls(), [], script)


class ActivateTests(Harness):
    def test_requires_approval_flag(self):
        self.passing_snapshot()
        result = self.activate()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("operator-approval", result.stderr)
        self.assertEqual(self.mutating_calls(), [])

    def test_refuses_stale_snapshot(self):
        self.passing_snapshot()
        result = self.approved(now=NOW + 1801)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("stale", result.stderr)
        self.assertEqual(self.mutating_calls(), [])

    def test_accepts_snapshot_just_inside_the_window(self):
        self.passing_snapshot()
        self.assertEqual(self.approved(now=NOW + 1800).returncode, 0)

    def test_dry_run_touches_nothing(self):
        self.passing_snapshot()
        before = tree_digest(self.root), tree_digest(self.stub)
        result = self.activate("--dry-run")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("[dry-run] systemctl stop pi-health.service", result.stdout)
        self.assertIn("[dry-run] env DEBIAN_FRONTEND=noninteractive dpkg --force-confold -i /pkg/limeos_1.0.0_arm64.deb", result.stdout)
        self.assertEqual(self.mutating_calls(), [])
        self.assertEqual(before, (tree_digest(self.root), tree_digest(self.stub)))

    def test_happy_path_order_and_scope(self):
        self.passing_snapshot()
        result = self.approved()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        mutating = self.mutating_calls()
        self.assertEqual(mutating, ACTIVATE_STEPS)
        # Proxy is configured only after the frozen app is stopped.
        self.assertLess(mutating.index("systemctl stop pi-health.service"),
                        mutating.index("systemctl start caddy.service"))
        for line in self.calls():
            for protected in ("limeops", "limeos-agent", "flask-shutdown"):
                if line.startswith("systemctl") and protected in line:
                    self.assertTrue(line.split()[1] in ("is-active", "is-enabled", "list-unit-files"), line)
        self.assertFalse([l for l in self.calls() if l.startswith("docker") and "ps" not in l and "inspect" not in l])
        for name in FROZEN:
            self.assertEqual(self.unit_state(name), ("inactive", "disabled"))
        for name in RUST:
            self.assertEqual(self.unit_state(name), ("active", "enabled"))
        # The reader is started but never enabled (it has no Install section).
        self.assertEqual(self.unit_state(READER), ("active", "disabled"))
        self.assertNotIn(f"systemctl enable {READER}", self.calls())
        self.assertTrue((self.root / READER_SOCKET).is_socket())
        site = (self.root / "etc/caddy/conf.d/limeos-cutover.caddy").read_text()
        self.assertIn(f"{SITE} {{", site)
        self.assertIn("tls internal", site)
        self.assertNotIn("@SITE@", site)
        self.assertNotIn("@TLS@", site)
        self.assertIn("reverse_proxy 127.0.0.1:8003", site)
        self.assertIn("keepalive 20s", site)

    def test_operator_certificate_mode(self):
        self._write("/etc/ssl/lime.crt", "c")
        self._write("/etc/ssl/lime.key", "k")
        self.passing_snapshot()
        result = self.approved("--tls-cert", "/etc/ssl/lime.crt", "--tls-key", "/etc/ssl/lime.key")
        self.assertEqual(result.returncode, 0, result.stderr)
        site = (self.root / "etc/caddy/conf.d/limeos-cutover.caddy").read_text()
        self.assertIn("tls /etc/ssl/lime.crt /etc/ssl/lime.key", site)

    def test_rejects_origin_mismatch_before_changing_anything(self):
        self._write("/etc/limeos/core.json", '{"version":1,"listen":"127.0.0.1:8003","origin":"https://localhost"}\n')
        self.passing_snapshot()
        result = self.approved()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("origin", result.stderr)
        self.assertEqual(self.mutating_calls(), [])

    def test_rejects_missing_caddy_import_before_changing_anything(self):
        self._write("/etc/caddy/Caddyfile", ":80 {\n}\n")
        self.passing_snapshot()
        result = self.approved()
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.mutating_calls(), [])

    def test_rejects_injection_in_host(self):
        self.passing_snapshot()
        args = self.activate_args("--yes-i-have-operator-approval")
        args[0] = str(self.snap)
        args[args.index("--host") + 1] = "x}\nevil"
        result = self.run_script("activate.sh", *args, now=NOW + 60)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.mutating_calls(), [])

    def assert_rolled_back(self, result):
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertIn("ROLLBACK: running return.sh", result.stderr)
        for name in FROZEN:
            self.assertEqual(self.unit_state(name), ("active", "enabled"), name)
        for name in RUST:
            self.assertEqual(self.unit_state(name), ("inactive", "disabled"), name)
        self.assertEqual(self.unit_state("caddy.service"), ("inactive", "disabled"))
        self.assertFalse((self.root / "etc/caddy/conf.d/limeos-cutover.caddy").exists())
        self.assertIn(":8002 ", (self.stub / "ss_out").read_text())
        for line in self.calls():
            tool, _, rest = line.partition(" ")
            if tool == "docker":
                self.assertIn(rest.split(" ")[0], ("ps", "inspect"))
            if tool == "systemctl" and rest.split(" ")[0] in MUTATING_VERBS:
                for protected in ("limeops", "limeos-agent", "flask-shutdown"):
                    self.assertNotIn(protected, line)

    def test_failure_injection_rolls_back(self):
        points = [
            "systemctl stop limeos-metrics-collector.timer limeos-package-reconcile.timer",
            "systemctl stop pi-health.service",
            "systemctl stop pihealth-helper.service",
            "systemctl disable pi-health.service",
            "dpkg --force-confold -i /pkg/limeos_1.0.0_arm64.deb",
            "systemctl daemon-reload",
            "systemctl enable limeos-core.service",
            "systemctl start limeos-containerd.service",
            "systemctl start limeos-core.service",
            f"systemctl start {READER}",
            "caddy validate --adapter caddyfile --config /etc/caddy/Caddyfile",
            "systemctl enable caddy.service",
            "systemctl start caddy.service",
            "limeosctl --socket /run/limeos-core/core.sock status",
        ]
        for point in points:
            with self.subTest(point=point):
                self.setUp()
                self.passing_snapshot()
                self.inject(point)
                self.assert_rolled_back(self.approved())

    def test_signal_at_each_mutating_step_rolls_back(self):
        # The signal arrives while the step runs; the step finishes, then the trap rolls back.
        self.signal_matrix(attached=True)

    def test_signal_at_each_mutating_step_rolls_back_the_detached_run(self):
        # The stub signals its caller, which here is the detached run itself.
        self.signal_matrix(attached=False)

    def signal_matrix(self, attached):
        for point in ACTIVATE_STEPS:
            for sig in ("INT", "TERM", "HUP"):
                with self.subTest(point=point, signal=sig):
                    self.setUp()
                    self.passing_snapshot()
                    (self.stub / "signal.txt").write_text(f"{sig} {point}\n")
                    result = self.approved(attached=attached)
                    if not attached:  # the run's output arrives through the followed log
                        result = types.SimpleNamespace(
                            returncode=result.returncode, stdout=result.stdout, stderr=result.stdout + result.stderr)
                    self.assert_rolled_back(result)
                    self.assertIn(f"interrupted by SIG{sig}", result.stderr)
                    # The EXIT trap after the rollback must not run it again.
                    self.assertEqual(result.stderr.count("ROLLBACK: running return.sh"), 1)
                    self.assertNotIn("ROLLBACK INCOMPLETE", result.stderr)

    def test_terminal_ctrl_c_lets_each_step_finish_then_rolls_back(self):
        # Ctrl-C at the terminal signals the whole foreground process group. The step must not
        # be killed halfway (dpkg mid-install); the script's trap rolls back once it returns.
        for point, finished in (("dpkg --force-confold -i /pkg/limeos_1.0.0_arm64.deb", "pkg_version"),
                                ("systemctl start limeos-core.service", "core-started")):
            with self.subTest(point=point):
                self.setUp()
                self.passing_snapshot()
                hook = self.stub / "hook-start-limeos-core.service"
                hook.write_text(f"#!/usr/bin/env bash\ntouch '{self.stub}/core-started'\n")
                hook.chmod(0o755)
                (self.stub / "signal.txt").write_text(f"group:INT {point}\n")
                args = self.activate_args("--yes-i-have-operator-approval", "--attached")
                args[0] = str(self.snap)
                result = self.run_script("activate.sh", *args, now=NOW + 60, new_session=True)
                self.assert_rolled_back(result)
                self.assertIn("interrupted by SIGINT", result.stderr)
                # The stubbed step ran to the end before the rollback.
                self.assertTrue((self.stub / finished).exists(), finished)

    def test_terminal_ctrl_c_lets_the_caddy_site_install_finish_then_rolls_back(self):
        self.passing_snapshot()
        real = shutil.which("install")
        self.fake_tool("install", (
            'if [[ "$*" == *limeos-cutover.caddy* ]]; then kill -s INT -- "-$(ps -o pgid= -p "${PPID}" | tr -d " ")"; fi\n'
            f'"{real}" "$@" || exit\n'
            f'if [[ "$*" == *limeos-cutover.caddy* ]]; then touch "{self.stub}/site-installed"; fi'))
        args = self.activate_args("--yes-i-have-operator-approval", "--attached")
        args[0] = str(self.snap)
        result = self.run_script("activate.sh", *args, now=NOW + 60, new_session=True)
        self.assert_rolled_back(result)
        self.assertIn("interrupted by SIGINT", result.stderr)
        self.assertTrue((self.stub / "site-installed").exists())

    def test_a_step_that_never_returns_is_stopped_after_the_grace_period_then_rolled_back(self):
        self.passing_snapshot()
        self.block_at_core_start()  # never released: the step hangs
        (self.stub / "signal.txt").write_text("TERM systemctl start limeos-core.service\n")
        self.env["CUTOVER_STEP_GRACE"] = "2"
        started = time.monotonic()
        result = self.approved()
        self.assertLess(time.monotonic() - started, 30)
        self.assert_rolled_back(result)
        self.assertIn("letting the running step finish (up to 2s)", result.stderr)
        self.assertIn("still running after 2s: systemctl start limeos-core.service", result.stderr)
        self.assertIn("interrupted by SIGTERM", result.stderr)

    @unittest.skipIf(os.geteuid() == 0, "CUTOVER_KILL_SIGNAL is ignored when run as root")
    def test_a_step_that_outlives_its_kill_does_not_hold_the_rollback(self):
        # A dpkg that ignores TERM and, here, the last-resort signal too. The kernel always
        # honours SIGKILL, so the test swaps in SIGUSR2 (CUTOVER_KILL_SIGNAL, honoured only under this
        # harness) to stand for a process stuck in uninterruptible I/O that KILL cannot end in time.
        self.passing_snapshot()
        # It lets go of the test's pipes, so the test can read the script's output while it lingers.
        self.fake_tool("dpkg", "trap '' TERM USR2\nexec </dev/null >/dev/null 2>&1\nkill -s TERM \"${PPID}\"\n"
                               "while :; do command -p sleep 0.1; done")
        self.env.update({"CUTOVER_STEP_GRACE": "1", "CUTOVER_KILL_SIGNAL": "USR2"})
        started = time.monotonic()
        result = self.approved()
        self.assertLess(time.monotonic() - started, 45)
        self.assertRegex(result.stderr, r"step \(pid \d+\) has not exited after SIGUSR2: "
                                        r"env DEBIAN_FRONTEND=noninteractive dpkg .*; rolling back regardless")
        self.assertIn("ROLLBACK: running return.sh", result.stderr)
        # return.sh finds the step still alive and says exactly what to do.
        self.assertRegex(result.stderr, r"the activation's step \(pid \d+: env DEBIAN_FRONTEND=noninteractive dpkg [^)]*\) is still running")
        self.assertIn("dpkg --audit", result.stderr)
        self.assertIn("ROLLBACK INCOMPLETE", result.stderr)
        for name in FROZEN:
            self.assertEqual(self.unit_state(name), ("active", "enabled"), name)

    def test_an_inherited_kill_signal_cannot_change_a_real_run(self):
        script = f'source "{SCRIPTS}/lib.sh"; printf "%s\\n" "${{KILL_SIGNAL}}"'
        base = {k: v for k, v in os.environ.items()
                if k not in ("LIMEOS_CUTOVER_ROOT", "CUTOVER_ALLOW_NONROOT", "CUTOVER_KILL_SIGNAL")}
        harness = "USR2" if os.geteuid() != 0 else "KILL"
        for extra, expected in (({}, "KILL"),
                                ({"LIMEOS_CUTOVER_ROOT": "/"}, "KILL"),
                                ({"CUTOVER_ALLOW_NONROOT": "1"}, "KILL"),
                                ({"LIMEOS_CUTOVER_ROOT": str(self.root), "CUTOVER_ALLOW_NONROOT": "1"}, harness)):
            with self.subTest(extra=extra):
                env = {**base, **extra, "CUTOVER_KILL_SIGNAL": "STOP" if expected == "KILL" else "USR2"}
                out = subprocess.run(["bash", "-c", script], env=env, capture_output=True, text=True, check=True).stdout
                self.assertEqual(out.strip(), expected)
        self.assertNotIn("KILL_SIGNAL", (SCRIPTS / "return.sh").read_text())

    def test_signal_before_the_first_change_rolls_nothing_back(self):
        self.passing_snapshot()
        # The read-only validation of the rendered site, before any change.
        (self.stub / "signal.txt").write_text("TERM caddy validate --adapter caddyfile --config *tmp.*\n")
        result = self.approved()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("interrupted by SIGTERM", result.stderr)
        self.assertNotIn("ROLLBACK", result.stderr)
        self.assertEqual(self.mutating_calls(), [])

    def test_rollback_is_not_cut_short_by_a_second_signal(self):
        self.passing_snapshot()
        (self.stub / "signal.txt").write_text(
            "TERM systemctl start limeos-core.service\n"
            # Sent to return.sh while it rolls back; the ignored disposition is inherited.
            "TERM systemctl stop limeos-core.service\n"
            "HUP systemctl stop limeos-storaged.service\n")
        result = self.approved()
        self.assert_rolled_back(result)
        self.assertIn("return verified", result.stderr)

    def test_unexpected_exit_after_the_first_change_rolls_back(self):
        self.passing_snapshot()
        # The post-stop hashes cannot be written, so set -e ends the script mid-activation.
        (self.snap / "state.poststop.sha256").mkdir()
        result = self.approved()
        self.assert_rolled_back(result)
        self.assertIn("before activation completed", result.stderr)
        self.assertEqual(result.stderr.count("ROLLBACK: running return.sh"), 1)
        # The report names the failed command and its line, before the rollback starts.
        match = re.search(r"^ERROR: activate\.sh stopped unexpectedly \(exit 1\): 'fct_state_hashes strict "
                          r".*state\.poststop\.sha256\S*' failed with status 1 at activate\.sh:(\d+) in fct_main, "
                          r"called from activate\.sh:\d+ in main$", result.stderr, re.MULTILINE)
        self.assertIsNotNone(match, result.stderr)
        self.assertIn("fct_state_hashes strict >", script_line("activate.sh", int(match.group(1))))
        self.assertLess(match.start(), result.stderr.index("ROLLBACK: running return.sh"))

    def block_at_core_start(self):
        """Hold the run inside `systemctl start limeos-core.service` until the test releases it."""
        reached, go = self.stub / "reached", self.stub / "go"
        hook = self.stub / "hook-start-limeos-core.service"
        hook.write_text(
            "#!/usr/bin/env bash\n"
            f"python3 -c 'import os; print(os.getsid(0))' >'{reached}'\n"
            f"while [[ ! -e '{go}' ]]; do command -p sleep 0.05; done\n")
        hook.chmod(0o755)
        self.addCleanup(lambda: go.touch())
        return reached, go

    def popen_detached(self):
        args = self.activate_args("--yes-i-have-operator-approval")
        args[0] = str(self.snap)
        return subprocess.Popen(
            ["bash", str(SCRIPTS / "activate.sh"), *args], env={**self.env, "CUTOVER_NOW": str(NOW + 60)},
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, start_new_session=True)

    def test_detaches_by_default_and_logs_into_the_snapshot(self):
        self.passing_snapshot()
        reached, go = self.block_at_core_start()
        go.touch()
        result = self.approved(attached=False)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("running detached", result.stdout)
        self.assertIn("activate complete", result.stdout)  # followed from the log
        log = (self.snap / "activate.log").read_text()
        self.assertIn("activate complete", log)
        runs = [l for l in self.calls() if l.startswith("systemd-run ")]
        self.assertEqual(len(runs), 1, runs)
        self.assertTrue(runs[0].startswith(f"systemd-run --scope --unit={self.scope_unit()} --quiet --collect -- "), runs[0])
        # The run had its own session, so the terminal's hangup cannot reach it.
        self.assertNotEqual(int(reached.read_text()), os.getsid(0))
        for name in RUST:
            self.assertEqual(self.unit_state(name), ("active", "enabled"))

    # Found in the Debian 12 VM rehearsal: the follower passed the run's status on with `return`,
    # which the ERR trap took for a failure, so every refused, failed or rolled-back detached
    # activation ended with a spurious "activate.sh stopped unexpectedly ... 'return "${rc}"'".
    def test_a_refused_detached_run_is_not_reported_as_an_unexpected_stop(self):
        self.passing_snapshot()
        result = self.approved("--host", "other.lan", attached=False)
        output = result.stdout + result.stderr
        self.assertEqual(result.returncode, 1, output)
        self.assertIn("core.json origin must be https://other.lan:8002", output)
        self.assertNotIn("stopped unexpectedly", output)

    def test_a_rolled_back_detached_run_is_not_reported_as_an_unexpected_stop(self):
        self.passing_snapshot()
        (self.stub / "units" / f"{READER}.restarts").write_text("2\n")
        result = self.approved(attached=False)
        output = result.stdout + result.stderr
        self.assertEqual(result.returncode, 1, output)
        self.assertIn("health check failed: stack reader ready: was false", output)
        self.assertIn("ROLLBACK: running return.sh", output)
        self.assertNotIn("stopped unexpectedly", output)

    def test_detached_run_survives_the_terminal_hanging_up(self):
        self.passing_snapshot()
        reached, go = self.block_at_core_start()
        proc = self.popen_detached()
        self.wait_for(reached.exists)
        os.killpg(proc.pid, signal.SIGHUP)  # what a dropped SSH session does to the foreground job
        proc.communicate(timeout=30)
        self.assertNotEqual(proc.returncode, 0)
        log = self.snap / "activate.log"
        self.assertNotIn("activate complete", log.read_text())
        go.touch()
        self.wait_for(lambda: "activate complete" in log.read_text())
        self.assertNotIn("ROLLBACK", log.read_text())
        for name in RUST:
            self.assertEqual(self.unit_state(name), ("active", "enabled"))
        self.assertEqual(self.unit_state("caddy.service"), ("active", "enabled"))

    def test_ctrl_c_on_the_follower_rolls_the_detached_run_back(self):
        self.passing_snapshot()
        reached, go = self.block_at_core_start()
        proc = self.popen_detached()
        self.wait_for(reached.exists)
        os.killpg(proc.pid, signal.SIGINT)
        go.touch()
        stdout, stderr = proc.communicate(timeout=60)
        result = types.SimpleNamespace(returncode=proc.returncode, stdout=stdout, stderr=stdout + stderr)
        self.assert_rolled_back(result)
        self.assertEqual(proc.returncode, 1)
        self.assertIn("interrupted by SIGTERM", stdout)

    def test_term_or_hup_to_the_follower_ends_only_the_follower(self):
        for sig in (signal.SIGTERM, signal.SIGHUP):
            with self.subTest(signal=sig.name):
                self.setUp()
                self.passing_snapshot()
                reached, go = self.block_at_core_start()
                proc = self.popen_detached()
                self.wait_for(reached.exists)
                proc.send_signal(sig)  # the follower only, as logind or sudo would
                stdout, _ = proc.communicate(timeout=30)
                self.assertNotEqual(proc.returncode, 0)
                self.assertIn("stopped following", stdout)
                go.touch()
                log = self.snap / "activate.log"
                self.wait_for(lambda: "activate complete" in log.read_text())
                self.assertNotIn("ROLLBACK", log.read_text())
                for name in RUST:
                    self.assertEqual(self.unit_state(name), ("active", "enabled"))

    def test_activate_refuses_missing_requirements_before_any_change(self):
        for tool in ("flock", "setsid", "env"):
            with self.subTest(tool=tool):
                self.setUp()
                self.passing_snapshot()
                if tool == "env":
                    self.env_without_default_signal()
                    fragment = "env does not support --default-signal"
                else:
                    self.env["PATH"] = self.path_without(tool)
                    fragment = f"{tool} is not installed"
                result = self.approved(attached=False)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn(fragment, result.stdout + result.stderr)
                self.assertEqual(self.mutating_calls(), [])

    def test_an_activation_fails_closed_without_a_scope_attached_or_detached(self):
        for case, attached in (("absent", False), ("failing", False), ("absent", True), ("failing", True)):
            with self.subTest(case=case, attached=attached):
                self.setUp()
                self.passing_snapshot()
                if case == "absent":
                    self.env["PATH"] = self.path_without("systemd-run")
                else:
                    self.fake_tool("systemd-run", "echo 'Failed to start transient scope unit: Access denied' >&2; exit 1")
                result = self.approved(attached=attached)
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(self.mutating_calls(), [])
                expected = "systemd-run is not installed" if case == "absent" else "Failed to start transient scope unit"
                self.assertIn(expected, result.stdout + result.stderr)
                for name in FROZEN:
                    self.assertEqual(self.unit_state(name), ("active", "enabled"))

    def test_a_rerun_after_a_completed_activation_changes_nothing(self):
        self.passing_snapshot()
        self.assertEqual(self.approved().returncode, 0)
        before = (tree_digest(self.root, skip=("calls.log", "activate.log", "activate.lock")),
                  tree_digest(self.stub / "units"))
        (self.stub / "calls.log").write_text("")
        for attached in (True, False):
            with self.subTest(attached=attached):
                result = self.approved(attached=attached)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("already activated", result.stdout + result.stderr)
                self.assertNotIn("ROLLBACK", result.stdout + result.stderr)
                self.assertEqual(self.mutating_calls(), [])
                self.assertEqual(before, (tree_digest(self.root, skip=("calls.log", "activate.log", "activate.lock")),
                                          tree_digest(self.stub / "units")))

    def test_an_active_cutover_site_refuses_a_new_activation(self):
        self.passing_snapshot()
        self._write("/etc/caddy/conf.d/limeos-cutover.caddy", "site from an earlier activation\n")
        result = self.approved()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("the cutover is already active", result.stderr)
        self.assertEqual(self.mutating_calls(), [])

    def test_a_second_run_is_refused_while_the_first_holds_the_lock(self):
        for attached in (True, False):
            with self.subTest(first_attached=attached):
                self.setUp()
                self.passing_snapshot()
                reached, go = self.block_at_core_start()
                args = self.activate_args("--yes-i-have-operator-approval", *(["--attached"] if attached else []))
                args[0] = str(self.snap)
                first = subprocess.Popen(
                    ["bash", str(SCRIPTS / "activate.sh"), *args], env={**self.env, "CUTOVER_NOW": str(NOW + 60)},
                    stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, start_new_session=True)
                self.wait_for(reached.exists)
                if not attached:  # the follower's terminal goes away; the detached run keeps the lock
                    os.killpg(first.pid, signal.SIGHUP)
                    first.communicate(timeout=30)
                calls_before = len(self.mutating_calls())
                second = self.approved()
                self.assertNotEqual(second.returncode, 0)
                self.assertRegex(second.stderr, r"activate\.sh \(pid \d+\) is running on this snapshot")
                self.assertEqual(len(self.mutating_calls()), calls_before)
                go.touch()
                log = self.snap / "activate.log"
                if attached:
                    out, err = first.communicate(timeout=60)
                    self.assertEqual(first.returncode, 0, out + err)
                else:
                    self.wait_for(lambda: "activate complete" in log.read_text())
                for name in RUST:
                    self.assertEqual(self.unit_state(name), ("active", "enabled"))

    def test_a_rollback_that_cannot_verify_says_rollback_incomplete(self):
        self.passing_snapshot()
        # Health fails, so activate rolls back; the frozen app then does not answer on 8002,
        # so return.sh restores the units but cannot verify.
        self.set_curl({SESSIONS: "200 []", f"{SITE}/api/v1/health": "503", "http://127.0.0.1:8002/": "503"})
        result = self.approved()
        self.assert_rolled_back(result)
        self.assertEqual(result.returncode, 1)
        self.assertIn("port 8002 does not serve the frozen app", result.stderr)
        self.assertIn("ROLLBACK INCOMPLETE: follow docs/p06-cutover-runbook.md", result.stderr)

    def test_return_refuses_while_an_activation_holds_the_lock(self):
        self.passing_snapshot()
        reached, go = self.block_at_core_start()
        args = self.activate_args("--yes-i-have-operator-approval", "--attached")
        args[0] = str(self.snap)
        first = subprocess.Popen(
            ["bash", str(SCRIPTS / "activate.sh"), *args], env={**self.env, "CUTOVER_NOW": str(NOW + 60)},
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, start_new_session=True)
        self.wait_for(reached.exists)
        calls_before = len(self.mutating_calls())
        result = self.do_return()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(f"activate.sh (pid {first.pid}) is running on this snapshot in scope {self.scope_unit()}", result.stderr)
        self.assertIn(f"kill -TERM {first.pid}", result.stderr)
        self.assertIn("return.sh --force", result.stderr)
        self.assertEqual(len(self.mutating_calls()), calls_before)
        # (--force is covered by test_forced_return_ends_the_activation_by_its_scope.)
        go.touch()
        out, err = first.communicate(timeout=60)
        self.assertEqual(first.returncode, 0, out + err)

    def stalled_activation(self, hook="", unkillable=False, popen=None):
        """An attached activation stuck for good inside `systemctl start limeos-core.service`.

        HOOK runs first inside the stalled step. With UNKILLABLE the step's process cannot be ended
        by the stub's systemctl kill (a process in uninterruptible I/O). POPEN replaces how the
        activation is started (it gets the command and the environment)."""
        self.passing_snapshot()
        reached = self.stub / "reached"
        loop = f"touch '{reached}'; while :; do command -p sleep 0.05; done"
        if unkillable:
            loop = f'exec env STUB_UNKILLABLE=1 bash -c "{loop}"'
        script = self.stub / "hook-start-limeos-core.service"
        script.write_text(f"#!/usr/bin/env bash\n{hook}\n{loop}\n")
        script.chmod(0o755)
        args = self.activate_args("--yes-i-have-operator-approval", "--attached")
        args[0] = str(self.snap)
        command = ["bash", str(SCRIPTS / "activate.sh"), *args]
        env = {**self.env, "CUTOVER_NOW": str(NOW + 60)}
        if popen:
            first = popen(command, env)
        else:
            first = subprocess.Popen(command, env=env, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE,
                                     text=True, start_new_session=True)
        self.wait_for(reached.exists)
        return first

    def popen_return(self, *extra, env=None):
        return subprocess.Popen(["bash", str(SCRIPTS / "return.sh"), str(self.snap), *extra],
                                env={**self.env, **(env or {})}, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                text=True)

    def assert_returned(self):
        for name in FROZEN:
            self.assertEqual(self.unit_state(name), ("active", "enabled"), name)
        for name in RUST:
            self.assertEqual(self.unit_state(name), ("inactive", "disabled"), name)
        self.assertEqual(self.unit_state("caddy.service"), ("inactive", "disabled"))
        self.assertIn(":8002 ", (self.stub / "ss_out").read_text())

    def kill_line(self):
        """The fallback kill, through systemd."""
        return f"systemctl kill --kill-whom=all --signal=KILL {self.scope_unit()}"

    def cgroup_kills(self):
        """Scopes the stub "kernel" killed through a write to their cgroup.kill."""
        path = self.stub / "cgroup-kills.log"
        return path.read_text().splitlines() if path.exists() else []

    def killed(self):
        return bool(self.cgroup_kills()) or self.kill_line() in self.calls()

    def test_forced_return_kills_nothing_when_cgroup_kill_cannot_be_used_unless_asked(self):
        # The stub cgroup tree: no cgroup.kill (kernel before 5.14), a cgroup.kill that cannot be
        # written, and a cgroup tree that is not a cgroup2 mount. systemctl kill would signal pids one
        # by one, so it runs only with --allow-systemctl-kill-fallback.
        cases = (("no-cgroup-kill", "has no cgroup.kill (kernel before 5.14)"),
                 ("cgroup-kill-readonly", "could not write 1 to"),
                 ("cgroupfs-tmpfs", "is not on a cgroup2 file system"))
        for flag, reason in cases:
            if flag == "cgroup-kill-readonly" and os.geteuid() == 0:
                continue  # root writes a read-only file anyway
            with self.subTest(case=flag):
                self.setUp()
                first = self.stalled_activation()
                unit = self.scope_unit()
                (self.stub / flag).touch()
                calls_before = self.mutating_calls()
                refused = self.do_return("--force")
                self.assertEqual(refused.returncode, 1, refused.stdout + refused.stderr)
                err = refused.stderr
                self.assertIn(reason, err)
                self.assertIn("Nothing was killed or changed", err)
                self.assertIn(f"Inspect: systemctl status {unit}; systemd-cgls --unit {unit}", err)
                self.assertIn(f"systemctl kill --kill-whom=all --signal=KILL {unit}", err)
                self.assertIn(f"run: return.sh {self.snap}\n", err)
                self.assertIn(f"return.sh {self.snap} --force --allow-systemctl-kill-fallback", err)
                self.assertNotIn(self.kill_line(), self.calls())
                self.assertEqual(self.cgroup_kills(), [])
                self.assertEqual(self.mutating_calls(), calls_before)
                self.assertIsNone(first.poll(), "the activation was killed")
                self.assertFalse((self.snap / "force-takeover").exists())
                # With the opt-in, the fallback runs.
                result = self.do_return("--force", "--allow-systemctl-kill-fallback")
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertIn(reason, result.stderr)
                self.assertIn(f"using systemctl kill --kill-whom=all --signal=KILL {unit} instead", result.stderr)
                self.assertIn(self.kill_line(), self.calls())
                self.assertEqual(self.cgroup_kills(), [])
                self.assertEqual(first.wait(timeout=10), -signal.SIGKILL)
                self.assertEqual(self.marked_pids(self.stub), [])
                self.assert_returned()

    def test_the_kill_fallback_flag_needs_force(self):
        self.passing_snapshot()
        result = self.do_return("--allow-systemctl-kill-fallback")
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("--allow-systemctl-kill-fallback applies only with --force", result.stderr)
        self.assertEqual(self.mutating_calls(), [])
        usage = self.do_return("--help").stdout
        self.assertIn("--allow-systemctl-kill-fallback", usage)
        self.assertIn("pid", usage)

    def test_an_attached_activation_runs_in_its_named_scope(self):
        self.passing_snapshot()
        unit = self.scope_unit()
        result = self.approved()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        runs = [l for l in self.calls() if l.startswith("systemd-run ")]
        self.assertEqual(len(runs), 1, runs)  # once: no step leaves the scope through a nested systemd-run
        self.assertTrue(runs[0].startswith(f"systemd-run --scope --unit={unit} --quiet --collect -- bash "), runs[0])
        self.assertTrue(runs[0].endswith(f" --scope-unit {unit}"), runs[0])
        # Every change ran inside the scope, the steps in their own sessions included.
        scoped = [(scope, call) for scope, call in self.scoped_calls()
                  if call.split(" ")[0] in ("dpkg", "caddy", "install")
                  or (call.startswith("systemctl ") and call.split(" ")[1] in MUTATING_VERBS)]
        self.assertTrue(scoped)
        self.assertEqual({scope for scope, _ in scoped}, {unit}, scoped)
        role, pid, start, recorded = (self.snap / "activate.lock").read_text().split()
        self.assertEqual((role, recorded), ("activate", unit))
        self.assertTrue(pid.isdigit() and start.isdigit())

    def test_a_rollback_runs_inside_the_activation_scope(self):
        self.passing_snapshot()
        self.inject("systemctl start limeos-core.service")
        result = self.approved()
        self.assert_rolled_back(result)
        unit = self.scope_unit()
        rollback = [(scope, call) for scope, call in self.scoped_calls() if call.startswith("systemctl stop limeos-core")]
        self.assertEqual(rollback, [(unit, "systemctl stop limeos-core.service")])

    def test_scope_unit_is_internal(self):
        self.passing_snapshot()
        for unit, fragment in (("bogus.scope", "--scope-unit is internal"),
                               (self.scope_unit(), "not running inside its transient scope")):
            with self.subTest(unit=unit):
                result = self.approved("--scope-unit", unit)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn(fragment, result.stderr)
                self.assertEqual(self.mutating_calls(), [])

    def test_forced_return_ends_the_activation_by_its_scope(self):
        first = self.stalled_activation()
        unit = self.scope_unit()
        self.assertTrue(self.marked_pids(self.stub))
        # An inherited test signal must not reach a forced return: it always sends a real KILL.
        self.env["CUTOVER_KILL_SIGNAL"] = "STOP"
        result = self.do_return("--force")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn(f"--force: ending the activation, every process in scope {unit}", result.stderr)
        # Through the kernel's cgroup kill: no pid, and no systemd per-pid kill.
        self.assertIn(f"--force: killed every process in scope {unit} at once by writing 1 to "
                      f"{self.root}/sys/fs/cgroup/system.slice/{unit}/cgroup.kill", result.stderr)
        self.assertEqual(self.cgroup_kills(), [unit])
        self.assertNotIn(self.kill_line(), self.calls())
        # The activation and its stalled step are gone, and the activation never ran its own rollback.
        self.assertEqual(self.marked_pids(self.stub), [])
        _, err = first.communicate(timeout=10)
        self.assertEqual(first.returncode, -signal.SIGKILL)
        self.assertNotIn("ROLLBACK", err)
        self.assertEqual((self.snap / "activate.lock").read_text().split()[0], "return")
        self.assertFalse((self.snap / "force-takeover").exists())
        self.assert_returned()

    def test_forced_return_ends_steps_started_after_it_began(self):
        # The stalled step keeps starting processes, each in a new session of its own, so some are
        # started after the forced return has begun. The scope holds them all.
        spawner = (f"while :; do setsid bash -c 'while :; do command -p sleep 0.05; done' </dev/null >/dev/null 2>&1 & "
                   "command -p sleep 0.02; done &")
        self.stalled_activation(hook=spawner)
        self.wait_for(lambda: len(self.marked_pids(self.stub)) > 8)
        result = self.do_return("--force")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(self.marked_pids(self.stub), [])
        self.assert_returned()

    def test_forced_return_leaves_processes_outside_the_scope_alone(self):
        # A session-leading shell starts a backup, then execs an attached activation: the backup
        # shares the activation's session and process group but is not in its scope.
        backup_pid = self.stub / "backup.pid"
        backup = f"bash -c 'while :; do command -p sleep 0.05; done' & echo $! >'{backup_pid}'; "

        def shell(command, env):
            return subprocess.Popen(["bash", "-c", backup + 'exec "$@"', "_", *command], env=env,
                                    stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True, start_new_session=True)
        first = self.stalled_activation(popen=shell)
        pid = int(backup_pid.read_text())
        self.assertEqual(os.getsid(pid), first.pid)
        result = self.do_return("--force")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(first.wait(timeout=10), -signal.SIGKILL)
        self.assertTrue(self.alive(pid), "the backup outside the scope was killed")
        self.assertEqual(self.pids_with(b"STUB_SCOPE=" + self.scope_unit().encode()), [])
        self.assert_returned()

    def test_forced_return_stops_when_the_scope_does_not_empty(self):
        first = self.stalled_activation(unkillable=True)
        unit = self.scope_unit()
        # The hook's shell (its sleep children come and go by themselves).
        survivors = [pid for pid in self.pids_with(b"STUB_UNKILLABLE=1")
                     if Path(f"/proc/{pid}/comm").read_text().strip() == "bash"]
        self.assertEqual(len(survivors), 1, survivors)
        # A member that exits while the list is being made: its pid is in cgroup.procs, its process gone.
        gone = subprocess.Popen(["true"])
        gone.wait()
        (self.stub / "cgroup-extra").write_text(f"{gone.pid}\n")
        calls_before = self.mutating_calls()
        self.env["CUTOVER_FORCE_WAIT"] = "1"
        result = self.do_return("--force")
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        err = result.stderr
        self.assertIn(f"--force: the activation's scope {unit} still has processes 1s after SIGKILL", err)
        self.assertIn(f"  {gone.pid} (exited while this list was made)", err)
        self.assertRegex(err, rf"\n +{survivors[0]} +\S+ +bash -c ")
        self.assertIn("The activation's automatic rollback was ended with it, and the old service is still stopped", err)
        self.assertIn(f"systemctl status {unit}", err)
        self.assertIn(f"Next: wait until 'systemctl is-active {unit}' prints inactive, then run: "
                      f"return.sh {self.snap} --force", err)
        # The only change was the kill; the activation itself is gone and could not roll back.
        self.assertEqual(self.mutating_calls(), calls_before)
        self.assertEqual(self.cgroup_kills(), [unit])
        self.assertEqual(first.wait(timeout=10), -signal.SIGKILL)
        self.assertTrue(all(self.alive(pid) for pid in survivors))
        self.assertTrue((self.snap / "force-takeover").exists())
        for name in FROZEN:
            self.assertEqual(self.unit_state(name), ("inactive", "disabled"), name)

        # After the refusal: a plain return explains instead of restoring beside the survivor, and
        # a new activation of this snapshot is refused.
        calls_before = self.mutating_calls()
        plain = self.do_return()
        self.assertEqual(plain.returncode, 1, plain.stdout + plain.stderr)
        self.assertIn("a forced return (return.sh --force, pid", plain.stderr)
        self.assertIn(f"did not finish: its scope {unit} still has processes", plain.stderr)
        self.assertIn(f"Next: wait until 'systemctl is-active {unit}' prints inactive, then run: "
                      f"return.sh {self.snap} --force", plain.stderr)
        self.assertNotIn("stale", plain.stderr)
        again = self.approved()
        self.assertNotEqual(again.returncode, 0)
        self.assertIn("force-takeover", again.stderr)
        self.assertEqual(self.mutating_calls(), calls_before)

        # Once the survivor has exited, a plain return finishes the job.
        for pid in survivors:
            os.kill(pid, signal.SIGKILL)
        self.wait_for(lambda: not self.pids_with(b"STUB_UNKILLABLE=1"))
        done = self.do_return()
        self.assertEqual(done.returncode, 0, done.stdout + done.stderr)
        self.assertIn(f"scope {unit} is now empty", done.stdout)
        self.assertFalse((self.snap / "force-takeover").exists())
        self.assert_returned()

    def test_a_concurrent_return_is_refused_for_the_whole_takeover(self):
        first = self.stalled_activation(unkillable=True)
        forced = self.popen_return("--force", env={"CUTOVER_FORCE_WAIT": "60"})
        self.wait_for(self.killed)
        # activate.sh is dead and its lock free by now; return.lock still keeps everyone out.
        self.assertEqual(first.wait(timeout=10), -signal.SIGKILL)
        calls_before = self.mutating_calls()
        for extra in ((), ("--force",)):
            with self.subTest(extra=extra):
                second = self.do_return(*extra)
                self.assertEqual(second.returncode, 1, second.stdout + second.stderr)
                self.assertIn(f"return.sh (pid {forced.pid}) is running on this snapshot", second.stderr)
        self.assertEqual(self.mutating_calls(), calls_before)
        for pid in self.pids_with(b"STUB_UNKILLABLE=1"):
            os.kill(pid, signal.SIGKILL)
        out, err = forced.communicate(timeout=60)
        self.assertEqual(forced.returncode, 0, out + err)
        self.assertFalse((self.snap / "force-takeover").exists())
        self.assert_returned()

    def test_a_plain_return_after_an_interrupted_forced_return_explains_it(self):
        self.stalled_activation(unkillable=True)
        unit = self.scope_unit()
        forced = self.popen_return("--force", env={"CUTOVER_FORCE_WAIT": "60"})
        self.wait_for(self.killed)
        forced.kill()  # the operator's terminal went away mid-takeover
        forced.communicate(timeout=10)
        self.wait_for(lambda: self.lock_free("return.lock"))  # its short-lived children have gone too
        calls_before = self.mutating_calls()
        plain = self.do_return()
        self.assertEqual(plain.returncode, 1, plain.stdout + plain.stderr)
        self.assertIn(f"a forced return (return.sh --force, pid {forced.pid}", plain.stderr)
        self.assertIn(f"its scope {unit} still has processes", plain.stderr)
        self.assertIn("the old service is still stopped", plain.stderr)
        self.assertIn(f"return.sh {self.snap} --force", plain.stderr)
        self.assertEqual(self.mutating_calls(), calls_before)

    def test_a_return_refuses_while_the_scope_still_has_processes_and_force_kills_nothing(self):
        # activate.sh died (here: SIGKILL from outside) and released its lock, but its step lives on.
        # With the lock free there is no live activation whose identity --force could check, so it
        # kills nothing either and names the command that would.
        first = self.stalled_activation()
        unit = self.scope_unit()
        first.kill()
        first.wait(timeout=10)  # (its step still holds the stderr pipe)
        self.wait_for(lambda: self.lock_free("activate.lock"))
        # The stalled step's long-lived processes (its sleep children come and go by themselves).
        step = [pid for pid in self.pids_with(b"STUB_SCOPE=" + unit.encode())
                if Path(f"/proc/{pid}/comm").read_text().strip() != "sleep"]
        self.assertTrue(step)
        calls_before = self.mutating_calls()
        for extra in ((), ("--force",)):
            with self.subTest(extra=extra):
                result = self.do_return(*extra)
                self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
                self.assertIn(f"activate.sh is no longer running, but its scope {unit} still has processes", result.stderr)
                self.assertIn(f"systemctl kill --kill-whom=all --signal=KILL {unit}", result.stderr)
                self.assertNotIn("--force: ending the activation", result.stderr)
        self.assertEqual(self.mutating_calls(), calls_before)
        self.assertTrue(all(self.alive(pid) for pid in step))
        # Once the step has gone, a plain return completes.
        self.kill_strays(self.stub)
        self.wait_for(lambda: not self.pids_with(b"STUB_SCOPE=" + unit.encode()))
        done = self.do_return()
        self.assertEqual(done.returncode, 0, done.stdout + done.stderr)
        self.assert_returned()

    def test_scope_names_differ_for_snapshots_sharing_a_basename(self):
        units = set()
        for parent in ("a", "b", "a/b", "c-d", "c_d"):
            directory = Path(self._tmp.name) / parent / "snap"
            directory.mkdir(parents=True)
            units.add(self.scope_unit(directory))
        self.assertEqual(len(units), 5, units)
        self.assertTrue(all(len(unit) <= 255 for unit in units))

    def test_an_activation_records_its_scope_in_the_snapshot(self):
        self.passing_snapshot()
        result = self.approved()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual((self.snap / "activation-scope").read_text(), self.scope_unit() + "\n")

    def test_forced_return_kills_nothing_unless_the_holder_is_inside_the_recorded_scope(self):
        self.passing_snapshot()
        unit = self.scope_unit()
        other = "limeos-cutover-activate-" + "0" * 32 + ".scope"
        (self.snap / "activation-scope").write_text(unit + "\n")
        member = self.scope_member(unit)  # the scope exists and is busy
        cases = (
            ("stale", "activate 4194000 123 " + unit + "\n", "is stale"),
            ("not an activation", self.own_record("return"), "--force overrides only an activation"),
            ("another unit", self.own_record("activate", other), f"records scope {other}, not this snapshot's {unit}"),
            ("outside the unit", self.own_record("activate", unit), f"is not inside {unit}"),
        )
        for label, record, fragment in cases:
            with self.subTest(case=label):
                handle = self.hold_lock_with_record(record)
                result = self.do_return("--force")
                handle.close()
                self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
                self.assertIn(fragment, result.stderr)
                self.assertIn("nothing was killed", result.stderr)
                self.assertEqual(self.mutating_calls(), [])
                self.assertFalse([l for l in self.calls() if l.startswith("systemctl kill")])
                self.assertTrue(self.alive(member.pid))
                self.assertFalse((self.snap / "force-takeover").exists())

    def test_a_slow_plain_return_cannot_strand_the_rollback(self):
        # After the frozen app has stopped, an operator's plain return.sh starts while the
        # activation is mid-step, and every systemd query it could make hangs past the old 60 s
        # limit. It must refuse at once (an activation holds the lock) without holding return.lock
        # across a query, so the activation's rollback, when its step then fails, still runs.
        self.passing_snapshot()
        reached, go = self.stub / "reached", self.stub / "go"
        hook = self.stub / "hook-start-limeos-core.service"
        hook.write_text(f"#!/usr/bin/env bash\ntouch '{reached}'\n"
                        f"while [[ ! -e '{go}' ]]; do command -p sleep 0.05; done\nexit 1\n")
        hook.chmod(0o755)
        self.addCleanup(go.touch)
        args = self.activate_args("--yes-i-have-operator-approval", "--attached")
        args[0] = str(self.snap)
        first = subprocess.Popen(["bash", str(SCRIPTS / "activate.sh"), *args], env={**self.env, "CUTOVER_NOW": str(NOW + 60)},
                                 stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, start_new_session=True)
        self.wait_for(reached.exists)
        self.hang("systemctl show *", "systemctl is-active *", "systemctl is-enabled *", "systemctl list-unit-files *")
        seen = len(self.calls())
        started = time.monotonic()
        plain = self.do_return()
        self.assertLess(time.monotonic() - started, 10)
        self.assertEqual(plain.returncode, 1, plain.stdout + plain.stderr)
        self.assertIn(f"activate.sh (pid {first.pid}) is running on this snapshot", plain.stderr)
        self.assertFalse([l for l in self.calls()[seen:] if l.startswith("systemctl ")], "the refusal made a systemd query")
        (self.stub / "hang.txt").unlink()
        go.touch()
        out, err = first.communicate(timeout=120)
        self.assert_rolled_back(types.SimpleNamespace(returncode=first.returncode, stdout=out, stderr=err))
        self.assertIn("return verified", err)

    def test_the_rollback_waits_for_a_return_holding_return_lock_and_says_who(self):
        self.passing_snapshot()
        self.inject("systemctl start limeos-core.service")
        handle = self.hold_lock_with_record(self.own_record("return"), name="return.lock")
        log = Path(self._tmp.name) / "activate.err"
        args = self.activate_args("--yes-i-have-operator-approval", "--attached")
        args[0] = str(self.snap)
        with open(log, "w") as err:
            first = subprocess.Popen(["bash", str(SCRIPTS / "activate.sh"), *args],
                                     env={**self.env, "CUTOVER_NOW": str(NOW + 60), "CUTOVER_LOCK_REPORT_INTERVAL": "1"},
                                     stdout=subprocess.DEVNULL, stderr=err, start_new_session=True)
        waiting = f"ROLLBACK: waiting for {self.snap}/return.lock, held by return.sh (pid {os.getpid()}, state "
        self.wait_for(lambda: log.read_text().count(waiting) >= 3)
        self.assertIsNone(first.poll())  # still waiting, not given up
        handle.close()
        self.assertEqual(first.wait(timeout=120), 1)
        text = log.read_text()
        self.assertIn(" since ", text)
        self.assertIn("return verified", text)
        self.assertNotIn("ROLLBACK INCOMPLETE", text)
        self.assert_returned()

    def test_a_hung_systemd_query_is_a_named_failure(self):
        self.passing_snapshot()
        self.assertEqual(self.approved().returncode, 0)
        unit = self.scope_unit()
        self.hang(f"systemctl is-active {unit}")
        self.env["CUTOVER_QUERY_TIMEOUT"] = "1"
        calls_before = self.mutating_calls()
        started = time.monotonic()
        result = self.do_return()
        self.assertLess(time.monotonic() - started, 30)
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn(f"timed out after 1s: systemctl is-active {unit}", result.stderr)
        self.assertIn(f"cannot tell whether the activation's scope {unit} is empty", result.stderr)
        self.assertEqual(self.mutating_calls(), calls_before)

    def test_a_timed_out_restart_count_fails_the_reader_check_and_rolls_back(self):
        # The restart count could not be read: the reader's crash history is unverified, so the check
        # fails after its retries, says it could not determine, and the activation rolls back.
        self.passing_snapshot()
        self.env["CUTOVER_QUERY_TIMEOUT"] = "1"
        self.hang(f"systemctl show -p NRestarts --value {READER}")
        result = self.approved(timeout=150)
        self.assertIn("health check failed: stack reader ready: could not determine", result.stderr)
        self.assertNotIn("was false", result.stderr)
        # The retries do not swallow the warning that names the query.
        self.assertIn(f"timed out after 1s: systemctl show -p NRestarts --value {READER}", result.stderr)
        self.assert_reader_stopped_by_rollback(result)

    def test_every_health_check_fails_on_an_unanswered_query(self):
        cases = (
            ("systemctl is-active limeos-core.service", "Rust services active"),
            ("docker inspect *", "managed containers: docker ps and StartedAt identical to the snapshot"),
            ("systemctl is-enabled limeops-worker.service", "untouched units unchanged"),
            (f"systemctl is-active {READER}", "stack reader ready"),
        )
        for pattern, label in cases:
            with self.subTest(query=pattern):
                self.setUp()
                self.passing_snapshot()
                self.silent(pattern)
                result = self.approved()
                self.assertIn(f"health check failed: {label}: could not determine", result.stderr)
                self.assert_rolled_back(result)

    def test_a_crash_looping_reader_is_reported_false_not_unknown(self):
        self.passing_snapshot()
        (self.stub / "units" / f"{READER}.restarts").write_text("2\n")
        result = self.approved()
        self.assertIn("health check failed: stack reader ready: was false", result.stderr)
        self.assertNotIn("could not determine", result.stderr)
        self.assert_reader_stopped_by_rollback(result)

    def test_an_unknown_caddy_state_on_activation_reloads_or_starts_it(self):
        self._unit("caddy.service", "active", "enabled")
        self.passing_snapshot()
        self.silent("systemctl is-active caddy.service")
        result = self.approved()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("could not determine whether caddy.service is running", result.stderr)
        self.assertIn("systemctl reload-or-restart caddy.service", self.calls())

    def test_an_unknown_installed_version_rolls_back_as_could_not_determine(self):
        self.passing_snapshot()
        self.silent("dpkg-query *")
        result = self.approved()
        self.assertIn("could not determine the installed limeos version", result.stderr)
        self.assert_rolled_back(result)

    def test_an_unknown_port_state_after_the_stop_rolls_back(self):
        self.passing_snapshot()
        self.silent("ss *")
        result = self.approved()
        self.assertIn("could not determine whether port 8002 is free", result.stderr)
        self.assert_rolled_back(result)

    def test_health_failures_roll_back(self):
        base = {
            "http://127.0.0.1:8096/Sessions": "401",
            f"{SITE}/api/v1/health": "200",
            f"{SITE}/": "200 <html>",
            "http://127.0.0.1:8002/": "200",
        }
        for key, value in ((f"{SITE}/api/v1/health", "503"), (f"{SITE}/", "502")):
            with self.subTest(url=key):
                self.setUp()
                self.passing_snapshot()
                self.set_curl({**base, key: value})
                self.assert_rolled_back(self.approved())

    def assert_reader_stopped_by_rollback(self, result):
        self.assert_rolled_back(result)
        self.assertEqual(self.unit_state(READER), ("inactive", "disabled"))
        self.assertIn(f"systemctl stop {READER}", self.calls())

    def test_reader_start_failure_rolls_back(self):
        self.passing_snapshot()
        self.inject(f"systemctl start {READER}")
        result = self.approved()
        self.assert_rolled_back(result)
        self.assertIn(f"command failed: systemctl start {READER}", result.stderr)
        self.assertEqual(self.unit_state(READER), ("inactive", "disabled"))

    def test_reader_that_never_listens_rolls_back_and_is_stopped(self):
        self.passing_snapshot()
        self.reader_listens(False)
        result = self.approved()
        self.assertIn("health check failed: stack reader ready", result.stderr)
        self.assert_reader_stopped_by_rollback(result)

    def test_reader_that_crash_loops_rolls_back_and_is_stopped(self):
        self.passing_snapshot()
        (self.stub / "units" / f"{READER}.restarts").write_text("3\n")
        result = self.approved()
        self.assertIn("health check failed: stack reader ready", result.stderr)
        self.assert_reader_stopped_by_rollback(result)

    def test_missing_catalog_is_refused_before_any_change(self):
        self.passing_snapshot()
        (self.root / CATALOG).unlink()
        result = self.approved()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("migrate propose-catalog --out", result.stderr)
        self.assertEqual(self.mutating_calls(), [])

    def test_a_signal_during_a_dry_run_never_runs_the_rollback(self):
        self.passing_snapshot()
        args = self.activate_args("--dry-run", "--attached")
        args[0] = str(self.snap)
        env = {**self.env, "CUTOVER_NOW": str(NOW + 60)}
        command = ["bash", str(SCRIPTS / "activate.sh"), *args]
        # Where the output stands once the first mutating step has been "run" (printed).
        full = subprocess.run(command, env=env, capture_output=True, timeout=60).stdout
        marker = b"[dry-run] systemctl stop limeos-metrics-collector.timer limeos-package-reconcile.timer\n"
        offset = full.index(marker) + len(marker)
        self.assertLess(offset, len(full))
        (self.stub / "calls.log").write_text("")
        # A one-page pipe as stdout (Linux rounds a pipe up to whole pages: 4 KiB here, 16 KiB on
        # the Pi), pre-filled so exactly `offset` bytes still fit: the script writes up to and
        # including that step, then blocks on its next line.
        r, w = os.pipe()
        self.addCleanup(os.close, r)
        page = os.sysconf("SC_PAGE_SIZE")
        fcntl.fcntl(w, 1031, page)  # F_SETPIPE_SZ
        size = fcntl.fcntl(w, 1032)  # F_GETPIPE_SZ
        self.assertEqual(size, page)  # one buffer: writes merge into it until it is full
        self.assertGreater(size, offset)
        os.write(w, b"j" * (size - offset))
        proc = subprocess.Popen(command, env=env, stdout=w, stderr=subprocess.PIPE)
        os.close(w)

        def pipe_full():
            import array
            avail = array.array("i", [0])
            fcntl.ioctl(r, 0x541B, avail)  # FIONREAD
            return avail[0] == size
        self.wait_for(pipe_full)
        proc.send_signal(signal.SIGTERM)
        while os.read(r, 65536):
            pass
        stderr = proc.communicate(timeout=60)[1].decode()
        self.assertNotEqual(proc.returncode, 0)
        self.assertIn("interrupted by SIGTERM", stderr)
        self.assertNotIn("ROLLBACK", stderr)
        self.assertEqual(self.mutating_calls(), [])
        for name in FROZEN:
            self.assertEqual(self.unit_state(name), ("active", "enabled"), name)

    def test_dry_run_prints_the_reader_start_and_check(self):
        self.passing_snapshot()
        result = self.activate("--dry-run")
        self.assertIn(f"[dry-run] systemctl start {READER}", result.stdout)
        self.assertIn("[dry-run] check stack reader ready", result.stdout)
        self.assertEqual(self.unit_state(READER), ("inactive", "disabled"))

    def test_version_mismatch_rolls_back(self):
        self.passing_snapshot()
        (self.stub / "deb_version").write_text("1.0.0\n")  # the file says 1.0.0, dpkg ends up with 9.9.9
        (self.stub / "pkg_to_install").write_text("9.9.9\n")
        self.assert_rolled_back(self.approved())

    def test_installs_only_the_named_deb_without_prompting(self):
        self._write("/pkg/limeos-shadow_1.0.0_arm64.deb", "other deb")
        self._write("/pkg/limeos_0.9.0_arm64.deb", "older deb")
        self.passing_snapshot()
        result = self.approved()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual([l for l in self.calls() if l.startswith("dpkg ")],
                         ["dpkg --force-confold -i /pkg/limeos_1.0.0_arm64.deb"])
        # What the stub can check: dpkg ran non-interactively and was told to keep existing conffiles.
        # Whether real dpkg then keeps a differing /etc/limeos/core.json without a prompt is not modelled.
        self.assertEqual((self.stub / "dpkg_frontend").read_text().strip(), "noninteractive")
        self.assertIn(f"dpkg-deb -f {self.root}/pkg/limeos_1.0.0_arm64.deb Version", self.calls())

    def test_rejects_a_package_directory_before_changing_anything(self):
        self.passing_snapshot()
        args = self.activate_args("--yes-i-have-operator-approval", "--attached")
        args[0] = str(self.snap)
        args[args.index("--package") + 1] = "/pkg"
        result = self.run_script("activate.sh", *args, now=NOW + 60)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(".deb file", result.stderr)
        self.assertEqual(self.mutating_calls(), [])

    def test_rejects_a_deb_of_another_version_or_package_before_changing_anything(self):
        for name, value, fragment in (("deb_version", "9.9.9", "is version 9.9.9, not 1.0.0"),
                                      ("deb_package", "limeos-shadow", "is package limeos-shadow, not limeos")):
            with self.subTest(field=name):
                self.setUp()
                self.passing_snapshot()
                (self.stub / name).write_text(value + "\n")
                result = self.approved()
                self.assertNotEqual(result.returncode, 0)
                self.assertIn(fragment, result.stderr)
                self.assertEqual(self.mutating_calls(), [])

    def test_a_hash_failure_after_the_writers_stop_rolls_back(self):
        self.passing_snapshot()
        hook = self.stub / "hook-disable-limeos-package-reconcile.timer"
        self.fake_tool("sha256sum.later", "exit 1")
        hook.write_text(f"#!/usr/bin/env bash\nmv '{self.bin}/sha256sum.later' '{self.bin}/sha256sum'\n")
        hook.chmod(0o755)
        # The rollback's own hashing must work again: return.sh stops limeos-core first.
        undo = self.stub / "hook-stop-limeos-core.service"
        undo.write_text(f"#!/usr/bin/env bash\nrm -f '{self.bin}/sha256sum'\n")
        undo.chmod(0o755)
        result = self.approved()
        self.assert_rolled_back(result)
        self.assertIn("cannot hash /etc/limeos/credentials.env", result.stderr)
        self.assertIn("before activation completed", result.stderr)

    def test_snapshot_record_without_a_hash_is_refused_before_any_change(self):
        self.passing_snapshot()
        records = self.snap / "state.sha256"
        lines = records.read_text().splitlines()
        lines[0] = "S  " + lines[0].split(" ", 2)[2]  # flag, empty hash, path
        records.write_text("\n".join(lines) + "\n")
        result = self.approved()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("malformed state record", result.stderr)
        self.assertEqual(self.mutating_calls(), [])

    def test_an_unterminated_last_record_is_refused_before_any_change(self):
        self.passing_snapshot()
        records = self.snap / "state.sha256"
        records.write_text(records.read_text() + "S  /srv/config.json")  # malformed, and no newline
        result = self.approved()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("does not end in a newline", result.stderr)
        self.assertEqual(self.mutating_calls(), [])
        # A well-formed last record without its newline is refused too: the file was cut short.
        records.write_text(records.read_text().rsplit("\n", 1)[0])
        result = self.approved()
        self.assertIn("does not end in a newline", result.stderr)
        self.assertEqual(self.mutating_calls(), [])

    def test_container_change_rolls_back(self):
        self.passing_snapshot()
        snap_ps = self.snap / "docker-ps.txt"
        snap_ps.write_text(snap_ps.read_text().replace("0001", "ffff", 1))
        self.assert_rolled_back(self.approved())

    def restart_during_activation(self, name):
        """The container restarts (new StartedAt) while the Rust services start."""
        started = self.stub / "docker_started.txt"
        hook = self.stub / "hook-start-limeos-core.service"
        hook.write_text(
            "#!/usr/bin/env bash\n"
            f"sed -i '/ \\/{name} /s/2026-11-01T00:00:00Z/2026-11-19T03:00:00Z/' '{started}'\n")
        hook.chmod(0o755)

    def test_restart_outside_the_managed_set_is_reported_not_rolled_back(self):
        managed = Path(self._tmp.name) / "managed.txt"
        managed.write_text("".join(f"{n}\n" for n in MANAGED if n != "watchtower"))
        self.assertEqual(self.preflight("--managed-file", str(managed)).returncode, 0)
        self.restart_during_activation("watchtower")
        result = self.approved()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("container outside the managed set changed (not rolled back): watchtower", result.stderr)
        self.assertNotIn("ROLLBACK", result.stderr)
        # return.sh applies the same rule: reported, not a failure.
        result = self.do_return()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("container outside the managed set changed", result.stderr)
        self.assertIn("watchtower", result.stderr)

    def test_restart_of_a_managed_container_rolls_back_and_names_it(self):
        self.passing_snapshot()
        self.restart_during_activation("radarr")
        result = self.approved()
        self.assert_rolled_back(result)
        self.assertIn("(changed: radarr)", result.stderr)
        self.assertIn("container StartedAt differs from the snapshot (a restart happened): radarr", result.stderr)

    def test_state_change_before_stop_rolls_back(self):
        self.passing_snapshot()
        self._write("/etc/limeos/credentials.env", "PIHEALTH_USERS=holly:CHANGED\n")
        self.assert_rolled_back(self.approved())


class ReturnTests(Harness):
    def activated(self):
        self.passing_snapshot()
        result = self.approved()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        (self.stub / "calls.log").write_text("")

    def test_clean_return_restores_everything(self):
        self.activated()
        result = self.do_return()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        for name in FROZEN:
            self.assertEqual(self.unit_state(name), ("active", "enabled"), name)
        for name in RUST:
            self.assertEqual(self.unit_state(name), ("inactive", "disabled"))
        self.assertFalse((self.root / "etc/caddy/conf.d/limeos-cutover.caddy").exists())
        # Start order: helper, app, timers.
        starts = [l for l in self.calls() if l.startswith("systemctl start")]
        self.assertEqual(starts, [f"systemctl start {n}" for n in FROZEN])

    def test_stops_the_stack_reader_and_never_enables_it(self):
        # activate.sh starts the reader; return must not leave that root service running.
        self.activated()
        self.assertEqual(self.unit_state(READER), ("active", "disabled"))
        self.assertEqual(self.do_return().returncode, 0)
        self.assertIn(f"systemctl stop {READER}", self.calls())
        self.assertEqual(self.unit_state(READER), ("inactive", "disabled"))
        self.assertNotIn(f"systemctl enable {READER}", self.calls())
        # A second return finds it stopped and does not touch it again.
        (self.stub / "calls.log").write_text("")
        self.assertEqual(self.do_return().returncode, 0)
        self.assertNotIn(f"systemctl stop {READER}", self.calls())

    def test_an_unexpected_failure_is_named_with_its_command_and_line(self):
        # A tool that fails where return.sh does not expect it ends the run under set -e; the run
        # must say which command failed, on which line, and how it got there.
        self.activated()
        (self.bin / "paste").write_text("#!/bin/sh\nexit 1\n")
        (self.bin / "paste").chmod(0o755)
        result = self.do_return()
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        match = re.search(r"^ERROR: return\.sh stopped unexpectedly \(exit 1\): 'ids=.*paste -sd ' ' -\)\"' "
                          r"failed with status 1 at return\.sh:(\d+) in fct_verify_docker, "
                          r"called from return\.sh:(\d+) in fct_main, called from return\.sh:\d+ in main$",
                          result.stderr, re.MULTILINE)
        self.assertIsNotNone(match, result.stderr)
        self.assertIn('ids="$(fct_docker_diff_names', script_line("return.sh", int(match.group(1))))
        self.assertIn('fct_verify_docker "${snap}"', script_line("return.sh", int(match.group(2))))

    def test_idempotent(self):
        self.activated()
        self.assertEqual(self.do_return().returncode, 0)
        skip = ("calls.log", "scoped-calls.log", "activate.lock", "return.lock")  # each return records itself as holder
        digest = tree_digest(self.root, skip=skip), tree_digest(self.stub / "units")
        second = self.do_return()
        self.assertEqual(second.returncode, 0, second.stderr)
        self.assertEqual(digest, (tree_digest(self.root, skip=skip), tree_digest(self.stub / "units")))

    def test_rust_units_enabled_before_the_cutover_are_restored_not_disabled(self):
        for name in RUST:
            self._unit(name, "active", "enabled")
        self.activated()
        result = self.do_return()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        for name in RUST:
            self.assertEqual(self.unit_state(name), ("active", "enabled"), name)
            self.assertNotIn(f"systemctl stop {name}", self.calls())
            self.assertNotIn(f"systemctl disable {name}", self.calls())
        # The reader was never enabled and is still stopped by return.
        self.assertEqual(self.unit_state(READER), ("inactive", "disabled"))

    def test_rolls_back_an_older_snapshot_with_the_previous_rules(self):
        self.activated()
        (self.snap / "state-paths.txt").unlink()
        (self.snap / "managed.txt").unlink()
        result = self.do_return()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("no state-paths.txt", result.stderr)
        self.assertIn("no managed.txt", result.stderr)
        for name in FROZEN:
            self.assertEqual(self.unit_state(name), ("active", "enabled"), name)
        # Listed files are still compared (taken from the snapshot's records) ...
        self._write("/home/holly/pi-health/config/media_paths.json", "changed")
        result = self.do_return()
        self.assertIn("state file changed: /home/holly/pi-health/config/media_paths.json", result.stderr)
        # ... and every container is held to the snapshot, as before managed sets.
        self._write("/home/holly/pi-health/config/media_paths.json", '{"m":1}')
        started = self.stub / "docker_started.txt"
        started.write_text(started.read_text().replace("/watchtower 2026-11-01", "/watchtower 2026-11-19"))
        result = self.do_return()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("StartedAt differs from the snapshot (a restart happened): watchtower", result.stderr)

    def test_activate_refuses_an_older_snapshot(self):
        self.passing_snapshot()
        for name in ("state-paths.txt", "managed.txt"):
            with self.subTest(missing=name):
                (self.snap / name).rename(self.snap / f"{name}.away")
                result = self.approved()
                self.assertNotEqual(result.returncode, 0)
                self.assertIn(f"no {name}", result.stderr)
                self.assertEqual(self.mutating_calls(), [])
                (self.snap / f"{name}.away").rename(self.snap / name)

    def test_a_stale_holder_record_is_not_named_as_a_process_to_signal(self):
        own_start = Path(f"/proc/{os.getpid()}/stat").read_text().rsplit(") ", 1)[1].split()[19]
        for label, record in (("exited", "activate 4194000 123\n"),
                              ("reused", f"activate {os.getpid()} {int(own_start) + 1}\n")):
            with self.subTest(record=label):
                self.setUp()
                self.activated()
                self.hold_lock_with_record(record)
                result = self.do_return()
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("holder record (activate", result.stderr)
                self.assertIn("is stale", result.stderr)
                self.assertNotIn("kill -TERM", result.stderr)
                self.assertEqual(self.mutating_calls(), [])

    def test_a_running_return_is_named_as_the_holder(self):
        self.activated()
        reached, go = self.stub / "return-reached", self.stub / "return-go"
        hook = self.stub / "hook-stop-limeos-core.service"
        hook.write_text(f"#!/usr/bin/env bash\ntouch '{reached}'\nwhile [[ ! -e '{go}' ]]; do command -p sleep 0.05; done\n")
        hook.chmod(0o755)
        first = subprocess.Popen(["bash", str(SCRIPTS / "return.sh"), str(self.snap)], env=self.env,
                                 stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, start_new_session=True)
        self.wait_for(reached.exists)
        record = (self.snap / "activate.lock").read_text().split()
        self.assertEqual(record[:2], ["return", str(first.pid)])
        second = self.do_return()
        self.assertNotEqual(second.returncode, 0)
        self.assertIn(f"return.sh (pid {first.pid}) is running on this snapshot", second.stderr)
        self.assertNotIn("kill -TERM", second.stderr)
        go.touch()
        out, err = first.communicate(timeout=60)
        self.assertEqual(first.returncode, 0, out + err)

    def test_activate_records_role_pid_and_start_time(self):
        self.activated()
        role, pid, start, unit = (self.snap / "activate.lock").read_text().split()
        self.assertEqual((role, unit), ("activate", self.scope_unit()))
        self.assertTrue(pid.isdigit() and start.isdigit())

    def test_return_without_prior_activate(self):
        self.passing_snapshot()
        self.assertEqual(self.do_return().returncode, 0)

    def test_dry_run_touches_nothing(self):
        self.activated()
        before = tree_digest(self.root), tree_digest(self.stub)
        result = self.do_return("--dry-run")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("[dry-run]", result.stdout)
        self.assertEqual(self.mutating_calls(), [])
        self.assertEqual(before, (tree_digest(self.root), tree_digest(self.stub)))

    def test_detects_changed_state_hash(self):
        self.activated()
        self._write("/etc/limeos/credentials.env", "PIHEALTH_USERS=holly:CHANGED\n")
        result = self.do_return()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("state file changed: /etc/limeos/credentials.env", result.stderr)
        self.assertNotIn("CHANGED", result.stdout + result.stderr)

    def test_frozen_units_are_restored_even_when_hashing_fails(self):
        for tool, fragment in (("mktemp", None), ("sha256sum", "cannot hash")):
            with self.subTest(tool=tool):
                self.setUp()
                self.activated()
                self.fake_tool(tool)
                result = self.do_return()
                for name in FROZEN:
                    self.assertEqual(self.unit_state(name), ("active", "enabled"), name)
                self.assertIn(":8002 ", (self.stub / "ss_out").read_text())
                if fragment:
                    self.assertNotEqual(result.returncode, 0)
                    self.assertIn(fragment, result.stderr)
                    self.assertIn("cannot verify state file: /etc/limeos/credentials.env", result.stderr)

    def test_an_unterminated_last_record_fails_verification(self):
        self.activated()
        records = self.snap / "state.sha256"
        records.write_text(records.read_text() + "S  /srv/config.json")
        result = self.do_return()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("malformed state record in the snapshot: S  /srv/config.json", result.stderr)
        for name in FROZEN:
            self.assertEqual(self.unit_state(name), ("active", "enabled"), name)

    def test_one_unreadable_path_does_not_hide_the_others(self):
        self.activated()
        media = self.root / "home/holly/pi-health/config/media_paths.json"
        media.unlink()
        media.mkdir()  # now a directory: it cannot be hashed
        self._write("/etc/limeos/credentials.env", "PIHEALTH_USERS=holly:CHANGED\n")
        result = self.do_return()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("cannot verify state file: /home/holly/pi-health/config/media_paths.json", result.stderr)
        self.assertIn("state file changed: /etc/limeos/credentials.env", result.stderr)
        self.assertNotIn("verification skipped", result.stderr)
        for name in FROZEN:
            self.assertEqual(self.unit_state(name), ("active", "enabled"), name)

    def test_malformed_snapshot_record_fails_verification(self):
        self.activated()
        records = self.snap / "state.sha256"
        records.write_text(records.read_text().replace(" /etc/limeos/credentials.env", "", 1)
                           .replace("S ", "S  ", 1))
        result = self.do_return()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("malformed state record", result.stderr)
        for name in FROZEN:
            self.assertEqual(self.unit_state(name), ("active", "enabled"), name)

    def test_detects_deleted_state_file(self):
        self.activated()
        (self.root / "home/holly/pi-health/config/media_paths.json").unlink()
        self.assertNotEqual(self.do_return().returncode, 0)

    def test_detects_changed_container_id(self):
        self.activated()
        ps = self.stub / "docker_ps.txt"
        ps.write_text(ps.read_text().replace("0001", "beef", 1))
        result = self.do_return()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("docker ps IDs differ", result.stderr)

    def test_detects_restarted_container(self):
        self.activated()
        started = self.stub / "docker_started.txt"
        started.write_text(started.read_text().replace("2026-11-01", "2026-11-06", 1))
        result = self.do_return()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("StartedAt differs", result.stderr)

    def test_detects_frozen_port_not_serving(self):
        self.activated()
        self.set_curl({"http://127.0.0.1:8002/": "503"})
        result = self.do_return()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("port 8002", result.stderr)

    def test_a_live_file_back_at_its_preflight_contents_is_checked_against_the_baseline(self):
        # A at preflight, B when the writers stopped, A again on return: an update was lost.
        self.passing_snapshot()
        self._write("/var/lib/limeos/metrics.db", "B-written-before-the-stop")
        result = self.approved()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self._write("/var/lib/limeos/metrics.db", "sqlite-bytes-1")
        result = self.do_return()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("live state file changed after the writers stopped: /var/lib/limeos/metrics.db "
                      "(checked against the post-stop baseline)", result.stderr)

    def test_live_database_change_is_flagged_against_post_stop_baseline(self):
        self.activated()
        self._write("/var/lib/limeos/metrics.db", "written-by-a-late-writer")
        self.assertNotEqual(self.do_return().returncode, 0)

    def test_hashes_are_verified_before_the_frozen_writers_restart(self):
        # Found in the VM rehearsal: a restored timer fires at once and rewrites its SQLite file,
        # which the old order (restore, then hash) reported as a late writer.
        self.activated()
        hook = self.stub / "hook-start-limeos-metrics-collector.timer"
        hook.write_text(f"#!/usr/bin/env bash\nprintf 'collector ran' >'{self.root}/var/lib/limeos/metrics.db'\n")
        hook.chmod(0o755)
        result = self.do_return()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("return verified", result.stdout)

    def test_second_return_tolerates_live_databases_but_not_strict_files(self):
        self.activated()
        self.assertEqual(self.do_return().returncode, 0)
        self._write("/var/lib/limeos/metrics.db", "written-by-the-restored-app")
        again = self.do_return()
        self.assertEqual(again.returncode, 0, again.stdout + again.stderr)
        self.assertIn("frozen writers are running", again.stderr)
        self._write("/etc/limeos/credentials.env", "PIHEALTH_USERS=holly:CHANGED\n")
        self.assertNotEqual(self.do_return().returncode, 0)

    def test_an_unknown_caddy_state_still_reloads_it_and_says_so(self):
        # Scenario B: Caddy served other sites before the cutover; during the return its state query
        # times out. The reload that drops the cutover site must happen anyway.
        self._unit("caddy.service", "active", "enabled")
        self.activated()
        self.env["CUTOVER_QUERY_TIMEOUT"] = "1"
        self.hang("systemctl is-active caddy.service")
        result = self.do_return()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("timed out after 1s: systemctl is-active caddy.service", result.stderr)
        self.assertIn("could not determine whether caddy.service is running", result.stderr)
        self.assertIn("systemctl try-reload-or-restart caddy.service", self.calls())
        self.assertFalse((self.root / "etc/caddy/conf.d/limeos-cutover.caddy").exists())
        self.assertEqual(self.unit_state("caddy.service"), ("active", "enabled"))

    def test_an_unknown_reader_state_still_stops_it(self):
        self.activated()
        self.silent(f"systemctl is-active {READER}")
        result = self.do_return()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn(f"could not determine whether {READER} is running", result.stderr)
        self.assertIn(f"systemctl stop {READER}", self.calls())
        self.assertEqual(self.unit_state(READER), ("inactive", "disabled"))

    def test_a_corrective_action_that_times_out_is_reported_and_the_rest_still_runs(self):
        self.activated()
        self.env["CUTOVER_ACTION_TIMEOUT"] = "1"
        self.hang("systemctl stop caddy.service")
        started = time.monotonic()
        result = self.do_return()
        self.assertLess(time.monotonic() - started, 30)
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("timed out after 1s: systemctl stop caddy.service", result.stderr)
        self.assertIn("run by hand: systemctl stop caddy.service", result.stderr)
        self.assertIn("return incomplete", result.stderr)
        # Every later step still ran.
        self.assertIn("systemctl disable caddy.service", self.calls())
        self.assertFalse((self.root / "etc/caddy/conf.d/limeos-cutover.caddy").exists())
        self.assertEqual(self.unit_state(READER), ("inactive", "disabled"))
        for name in RUST:
            self.assertEqual(self.unit_state(name), ("inactive", "disabled"), name)
        for name in FROZEN:
            self.assertEqual(self.unit_state(name), ("active", "enabled"), name)
        self.assertIn("return: verifying", result.stdout)

    def test_unanswered_queries_during_verification_fail_it_by_name(self):
        def late_writer():
            self._write("/var/lib/limeos/metrics.db", "written-by-a-late-writer")
        cases = (
            ("docker ps *", None, "could not determine the containers' IDs and StartedAt"),
            ("systemctl is-active limeos-agent.service", None,
             "could not determine the state of untouched unit limeos-agent.service"),
            ("systemctl is-active pi-health.service", late_writer,
             "whether the frozen writers were running could not be determined"),
        )
        for pattern, setup, fragment in cases:
            with self.subTest(query=pattern):
                self.setUp()
                self.activated()
                if setup:
                    setup()
                self.silent(pattern)
                result = self.do_return()
                self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
                self.assertIn(fragment, result.stderr)
                self.assertNotIn("changed state", result.stderr)
                self.assertNotIn("IDs differ", result.stderr)
                for name in FROZEN:
                    self.assertEqual(self.unit_state(name), ("active", "enabled"), name)

    def test_live_database_difference_only_warns_without_baseline(self):
        self.passing_snapshot()
        self._write("/var/lib/limeos/metrics.db", "writer-ran-before-stop")
        result = self.do_return()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("live state file differs", result.stderr)


class MeasureTests(Harness):
    """measure.sh reads a fake /proc and /sys/fs/cgroup under the temp root."""

    OUT = "/var/lib/limeos-cutover/measure"
    # unit -> list of (pid, comm, pss_kB, swap_kB, start ticks, end ticks, start write_bytes, end write_bytes)
    PROCS = {
        "limeos-core.service": [(1201, "limeos-core", 9000, 0, 100, 110, 4096, 12288),
                                (1202, "tokio rt worker", 1000, 0, 5, 6, 0, 4096)],
        "limeos-storaged.service": [(1301, "limeos-executor", 5000, 0, 50, 55, 0, 8192)],
        "limeos-containerd.service": [(1401, "limeos-executor", 5000, 0, 40, 44, 0, 8192)],
        READER: [(1501, "limeos-executor", 6000, 0, 30, 33, 0, 8192)],
    }

    def setUp(self):
        super().setUp()
        self.tmpdir = Path(self._tmp.name) / "tmp"
        self.tmpdir.mkdir()
        self.env["TMPDIR"] = str(self.tmpdir)
        self.report_path = self.root / self.OUT.lstrip("/") / "report.json"
        self._write("/proc/uptime", "1000.00 3000.00\n")
        self._write("/proc/meminfo", "MemAvailable: 6000000 kB\nSwapTotal: 523264 kB\nSwapFree: 100000 kB\n")
        (self.root / "sys/fs/cgroup/system.slice/docker-abc.scope").mkdir(parents=True)
        self.end = self.stub / "proc-end"
        self._write_end("uptime", "1600.00 4000.00\n")
        for unit, procs in self.PROCS.items():
            self.seed_unit(unit, procs)
        hook = self.stub / "hook-sleep"
        hook.write_text(f"#!/usr/bin/env bash\ncp -R '{self.end}/.' '{self.root}/proc/'\n")
        hook.chmod(0o755)
        self._write("/usr/lib/limeos/limeos-core", "x" * 3000)
        self._write("/usr/lib/limeos/ui/index.html", "y" * 1000)
        (self.stub / "pkg_info.limeos").write_text("installed 1.0.0 40\n")
        (self.stub / "pkg_files.limeos").write_text(
            "/.\n/usr/lib/limeos\n/usr/lib/limeos/limeos-core\n/usr/lib/limeos/ui\n/usr/lib/limeos/ui/index.html\n")

    def _write_end(self, rel, text):
        target = self.end / rel
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(text)

    @staticmethod
    def _stat(pid, comm, ticks):
        return f"{pid} ({comm}) S 1 {pid} {pid} 0 -1 4194560 100 0 0 0 {ticks} 0 0 0 20 0 4 0 50000 123456 789\n"

    def seed_unit(self, unit, procs):
        cgroup = self.root / f"sys/fs/cgroup/system.slice/{unit}"
        cgroup.mkdir(parents=True, exist_ok=True)
        (cgroup / "cgroup.procs").write_text("".join(f"{p[0]}\n" for p in procs))
        (self.stub / "units" / f"{unit}.cgroup").write_text(f"/system.slice/{unit}\n")
        (self.stub / "units" / f"{unit}.mainpid").write_text(f"{procs[0][0]}\n")
        for pid, comm, pss, swap, t0, t1, w0, w1 in procs:
            self._write(f"/proc/{pid}/comm", comm + "\n")
            self._write(f"/proc/{pid}/stat", self._stat(pid, comm, t0))
            self._write(f"/proc/{pid}/io", f"rchar: 1\nwchar: 2\nread_bytes: 0\nwrite_bytes: {w0}\ncancelled_write_bytes: 0\n")
            self._write(f"/proc/{pid}/smaps_rollup",
                        f"00400000-7fff [rollup]\nRss: {pss * 2} kB\nPss: {pss} kB\nPss_Anon: {pss} kB\n"
                        f"Swap: {swap} kB\nSwapPss: {swap} kB\nLocked: 0 kB\n")
            self._write_end(f"{pid}/stat", self._stat(pid, comm, t1))
            self._write_end(f"{pid}/io", f"rchar: 1\nwchar: 2\nread_bytes: 0\nwrite_bytes: {w1}\ncancelled_write_bytes: 0\n")

    def remove_unit(self, unit):
        shutil.rmtree(self.root / f"sys/fs/cgroup/system.slice/{unit}")
        (self.stub / "units" / f"{unit}.cgroup").unlink()
        (self.stub / "units" / f"{unit}.mainpid").unlink()

    def measure(self, *extra):
        return self.run_script("measure.sh", "--host", HOST, "--out", self.OUT, *extra)

    def report(self):
        import json
        return json.loads(self.report_path.read_text())

    def rows(self):
        return {row["id"]: row for row in self.report()["measurements"]}

    def assert_read_only(self):
        self.assertEqual(self.mutating_calls(), [])
        self.assertFalse([l for l in self.calls() if l.startswith("docker")])
        self.assertFalse([l for l in self.calls() if l.startswith("systemctl") and l.split()[1] != "show"])

    def test_pass_reports_every_target(self):
        result = self.measure("--window", "600")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("sleep 600", self.calls())
        report = self.report()
        self.assertEqual(report["verdict"], "pass")
        self.assertTrue(report["read_only"])
        self.assertEqual(report["missing_units"], [])
        self.assertEqual(report["meta"]["docker_scopes"], "1")
        rows = self.rows()
        self.assertEqual(rows["core_base_idle_pss"]["value"], 20000 * 1024)
        self.assertEqual(rows["core_base_idle_pss"]["target"], 30 * 1024 * 1024)
        self.assertEqual(rows["core_base_idle_swap"]["value"], 0)
        self.assertEqual(rows["all_resident_idle_pss"]["value"], 26000 * 1024)
        # 10+1+5+4+3 ticks at 100 Hz over 600 s of uptime.
        self.assertEqual(rows["backend_cpu_idle"]["elapsed_s"], 600.0)
        self.assertAlmostEqual(rows["backend_cpu_idle"]["value"], 23 / 100 / 600 * 100, places=4)
        # 8192+4096+8192*3 bytes over 600 s, per day.
        self.assertAlmostEqual(rows["resident_disk_writes"]["value"], 36864 / 600 * 86400 / 1e6, places=3)
        self.assertEqual(rows["health_http_p95"]["value"], 2.0)  # 4 ms total minus 2 ms TLS setup
        self.assertEqual(rows["health_http_p95"]["requests"], 100)
        self.assertEqual(rows["installed_footprint"]["value"], 40960)  # Installed-Size beats 4000 file bytes
        self.assertEqual(rows["installed_footprint"]["packages"]["limeos"]["file_bytes"], 4000)
        for ident in ("core_base_idle_pss", "core_base_idle_swap", "all_resident_idle_pss", "backend_cpu_idle",
                      "resident_disk_writes", "health_http_p95", "installed_footprint"):
            self.assertEqual(rows[ident]["status"], "pass", ident)
        self.assertEqual(rows["cached_observation_http_p95"]["status"], "unmeasured")
        self.assertIn("--cookie-file", rows["cached_observation_http_p95"]["note"])
        self.assertEqual(rows["core_restart_readiness"]["status"], "unmeasured")
        self.assertIn("tokio_rt_worker", [p["comm"] for p in report["processes"]])
        self.assert_read_only()

    def test_writes_nothing_outside_the_output_path(self):
        (self.stub / "hook-sleep").unlink()  # a static /proc, so the whole root must stay identical
        before = tree_digest(self.root), tree_digest(self.stub)
        result = self.measure("--window", "1")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("sleep 1", self.calls())
        self.assertEqual(sorted(p.name for p in self.report_path.parent.iterdir()), ["raw.txt", "report.json"])
        shutil.rmtree(self.root / "var/lib/limeos-cutover")
        self.assertEqual(before, (tree_digest(self.root), tree_digest(self.stub)))
        self.assertEqual(list(self.tmpdir.iterdir()), [])
        self.assert_read_only()
        for line in self.calls():
            self.assertNotIn("/opt/stacks", line)
            self.assertNotIn("pi-health", line)

    def test_pss_over_target_fails_without_rollback(self):
        self.seed_unit("limeos-core.service", [(1201, "limeos-core", 40000, 0, 100, 110, 0, 0)])
        result = self.measure("--window", "5")
        self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
        self.assertIn("nothing was rolled back", result.stderr)
        self.assertEqual(self.report()["verdict"], "fail")
        rows = self.rows()
        self.assertEqual(rows["core_base_idle_pss"]["status"], "fail")
        self.assertEqual(rows["core_base_idle_swap"]["status"], "pass")
        self.assert_read_only()

    def test_swap_present_fails(self):
        self.seed_unit("limeos-containerd.service", [(1401, "limeos-executor", 5000, 128, 40, 44, 0, 0)])
        result = self.measure("--window", "5")
        self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
        rows = self.rows()
        self.assertEqual(rows["core_base_idle_swap"]["status"], "fail")
        self.assertEqual(rows["core_base_idle_swap"]["value"], 128 * 1024)
        self.assertEqual(rows["core_base_idle_pss"]["status"], "pass")
        self.assert_read_only()

    def test_missing_process_is_incomplete_not_a_pass(self):
        self.remove_unit("limeos-storaged.service")
        result = self.measure("--window", "5")
        self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
        report = self.report()
        self.assertEqual(report["verdict"], "incomplete")
        self.assertEqual(report["missing_units"], ["limeos-storaged.service"])
        rows = self.rows()
        for ident in ("core_base_idle_pss", "core_base_idle_swap", "backend_cpu_idle", "resident_disk_writes"):
            self.assertEqual(rows[ident]["status"], "unmeasured", ident)
            self.assertIn("limeos-storaged.service", rows[ident]["note"])
        self.assert_read_only()

    def test_optional_executor_absent_is_noted_not_missing(self):
        self.remove_unit(READER)
        result = self.measure("--window", "5")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        rows = self.rows()
        self.assertEqual(rows["all_resident_idle_pss"]["value"], 20000 * 1024)
        self.assertIn(READER, rows["all_resident_idle_pss"]["not_running_optional"])

    def test_mainpid_fallback_without_cgroup(self):
        (self.stub / "units" / "limeos-storaged.service.cgroup").unlink()
        result = self.measure("--window", "5")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn(1301, [p["pid"] for p in self.report()["processes"]])

    def test_cpu_and_disk_over_target_fail(self):
        # 0.1 % of one core over 600 s is 60 ticks; 20 MB/day over 600 s is about 139 kB.
        self.seed_unit("limeos-core.service", [(1201, "limeos-core", 9000, 0, 100, 200, 0, 500_000)])
        result = self.measure("--window", "600")
        self.assertEqual(result.returncode, 2)
        rows = self.rows()
        self.assertEqual(rows["backend_cpu_idle"]["status"], "fail")
        self.assertEqual(rows["resident_disk_writes"]["status"], "fail")

    def test_slow_or_failing_health_fails(self):
        (self.stub / "curl_times").write_text("0.050 0.010 0.001")
        self.measure("--window", "5", "--requests", "3")
        self.assertEqual(self.rows()["health_http_p95"]["status"], "fail")
        self.set_curl({f"{SITE}/api/v1/health": "503"})
        shutil.rmtree(self.root / "var/lib/limeos-cutover")
        self.measure("--window", "5", "--requests", "3")
        row = self.rows()["health_http_p95"]
        self.assertEqual(row["status"], "fail")
        self.assertIn("3 of 3 requests failed", row["note"])

    def test_cookie_file_measures_the_authenticated_read_without_copying_it(self):
        self._write("/root/session.cookies", "lime.lan\tFALSE\t/\tTRUE\t0\tsession\tCOOKIE-SECRET\n")
        self.set_curl({f"{SITE}/api/v1/health": "200", f"{SITE}/api/v1/overview": "200 {}"})
        result = self.measure("--window", "5", "--requests", "4", "--cookie-file", "/root/session.cookies")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        row = self.rows()["cached_observation_http_p95"]
        self.assertEqual(row["status"], "pass")
        self.assertEqual(row["requests"], 4)
        for path in self.report_path.parent.iterdir():
            self.assertNotIn("COOKIE-SECRET", path.read_text())
        self.assertIn("COOKIE-SECRET", (self.root / "root/session.cookies").read_text())

    def test_rejected_session_is_unmeasured(self):
        self._write("/root/session.cookies", "expired\n")
        self.set_curl({f"{SITE}/api/v1/health": "200", f"{SITE}/api/v1/overview": "401"})
        self.measure("--window", "5", "--requests", "2", "--cookie-file", "/root/session.cookies")
        row = self.rows()["cached_observation_http_p95"]
        self.assertEqual(row["status"], "unmeasured")
        self.assertIn("session rejected", row["note"])

    def test_core_restart_is_opt_in(self):
        self.measure("--window", "5")
        self.assertNotIn("systemctl restart limeos-core.service", self.calls())
        shutil.rmtree(self.root / "var/lib/limeos-cutover")
        result = self.measure("--window", "5", "--measure-core-restart")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("systemctl restart limeos-core.service", self.calls())
        report = self.report()
        self.assertFalse(report["read_only"])
        row = self.rows()["core_restart_readiness"]
        self.assertEqual(row["status"], "pass")
        self.assertIsInstance(row["value"], float)

    def test_an_unanswered_systemd_query_is_reported_unknown_not_a_value(self):
        # Before: a timed-out ControlGroup query fell back to MainPID and reported the main process's
        # numbers as the unit's, or reported the unit as not running.
        self.env["CUTOVER_QUERY_TIMEOUT"] = "1"
        self.hang("systemctl show -p ControlGroup --value limeos-core.service")
        result = self.measure("--window", "5")
        self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
        self.assertIn("timed out after 1s: systemctl show -p ControlGroup --value limeos-core.service", result.stderr)
        report = self.report()
        self.assertEqual(report["verdict"], "incomplete")
        self.assertEqual(report["missing_units"], [])
        self.assertEqual(report["unknown_units"], ["limeos-core.service"])
        rows = self.rows()
        for ident in ("core_base_idle_pss", "core_base_idle_swap", "all_resident_idle_pss",
                      "backend_cpu_idle", "resident_disk_writes"):
            self.assertEqual(rows[ident]["status"], "unknown", ident)
            self.assertIsNone(rows[ident]["value"], ident)
            self.assertIn("could not determine", rows[ident]["note"], ident)
        self.assertNotIn(1201, [p["pid"] for p in report["processes"]])
        self.assertIn("UNKNOWN", result.stdout)

    def test_an_unanswered_package_query_is_reported_unknown(self):
        self.silent("dpkg-query *")
        result = self.measure("--window", "5")
        self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
        row = self.rows()["installed_footprint"]
        self.assertEqual(row["status"], "unknown")
        self.assertIsNone(row["value"])
        self.assertIn("could not determine", row["note"])
        self.assertEqual(self.report()["verdict"], "incomplete")

    def test_rejects_bad_arguments_and_a_used_output_directory(self):
        for args in (("--window", "0"), ("--window", "ten"), ("--requests", "0"), ("--host", "x}\nevil"),
                     ("--cookie-file", "relative.txt")):
            with self.subTest(args=args):
                result = self.measure(*args)
                self.assertEqual(result.returncode, 1, result.stderr)
        self.assertEqual(self.run_script("measure.sh", "--out", self.OUT).returncode, 1)
        self.assertEqual(self.measure("--window", "5").returncode, 0)
        result = self.measure("--window", "5")
        self.assertEqual(result.returncode, 1)
        self.assertIn("not empty", result.stderr)
        self.assert_read_only()


class LintTests(unittest.TestCase):
    def test_ci_shellchecks_the_cutover_scripts(self):
        ci = (REPO / ".github/workflows/ci.yml").read_text()
        self.assertIn("run: shellcheck scripts/cutover/*.sh", ci)

    @unittest.skipUnless(shutil.which("shellcheck"), "shellcheck is not installed")
    def test_cutover_scripts_pass_shellcheck(self):
        scripts = sorted(str(p.relative_to(REPO)) for p in SCRIPTS.glob("*.sh"))
        result = subprocess.run(["shellcheck", *scripts], cwd=REPO, capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)


    def test_no_command_substitution_of_a_bare_redirection(self):
        # Under set -e, bash 5.2 (Debian 12, the production OS) ends the whole script when the
        # file of a $(<FILE) cannot be opened, whatever guards it (|| and if do not help, and the
        # ERR trap does not run): return.sh --force died silently that way once the scope it had
        # just killed was removed. bash 5.3 handles it, so only this check fails there.
        pattern = re.compile(r"\$\(\s*<")
        found = [f"{path.name}:{number}: {line.strip()}"
                 for path in sorted(SCRIPTS.glob("*.sh"))
                 for number, line in enumerate(path.read_text().splitlines(), 1)
                 if pattern.search(line) and not line.lstrip().startswith("#")]
        self.assertEqual(found, [], "read files with mapfile or read instead")


def script_line(name, number):
    """Line NUMBER (from 1) of scripts/cutover/NAME."""
    return (SCRIPTS / name).read_text().splitlines()[number - 1]


class ErrorReportTests(unittest.TestCase):
    """No cutover script stops with a non-zero status without naming what failed and where."""

    def run_bash(self, body, *args):
        with tempfile.TemporaryDirectory(prefix="cutover-err-") as tmp:
            script = Path(tmp) / "probe.sh"
            script.write_text("set -Eeuo pipefail\n"
                              f"source '{SCRIPTS / 'lib.sh'}'\n" + body)
            return subprocess.run(["bash", str(script), *args], capture_output=True, text=True, timeout=30)

    def test_every_script_reports_through_the_library(self):
        for path in sorted(SCRIPTS.glob("*.sh")):
            if path.name == "lib.sh":
                continue
            with self.subTest(script=path.name):
                text = path.read_text()
                self.assertIn("\nset -Eeuo pipefail\n", text)
                self.assertRegex(text, r'\nsource "\$\{SCRIPT_DIR\}/lib\.sh"\n')
                # Its own EXIT or ERR trap would replace the report.
                self.assertIsNone(re.search(r"^\s*trap\s.*\s(EXIT|ERR)\s*$", text, re.MULTILINE))
        lib = (SCRIPTS / "lib.sh").read_text()
        self.assertIn("""trap 'fct_on_err "$?" "${BASH_COMMAND}"' ERR""", lib)
        self.assertIn("""trap 'fct_exit_trap "$?" "${BASH_COMMAND}"' EXIT""", lib)

    def test_a_failure_under_set_e_is_named_with_its_line_and_callers(self):
        result = self.run_bash("inner() {\n    false\n}\nouter() {\n    inner\n}\nouter\n")
        self.assertEqual(result.returncode, 1)
        self.assertEqual(result.stderr,
                         "ERROR: probe.sh stopped unexpectedly (exit 1): 'false' failed with status 1 at probe.sh:4 "
                         "in inner, called from probe.sh:7 in outer, called from probe.sh:9 in main\n")

    def test_an_exit_that_skips_the_err_trap_is_still_named(self):
        # bash ends a script without running its ERR trap in some cases (an unset variable under
        # set -u, or on bash 5.2 a failed $(<FILE)); the report then names the command, the
        # function it was in and its callers.
        result = self.run_bash("inner() {\n    : \"${UNSET_FOR_TEST}\"\n}\nouter() {\n    inner\n}\nouter\n")
        self.assertEqual(result.returncode, 1)
        self.assertIn("ERROR: probe.sh stopped unexpectedly (exit 1): ': \"${UNSET_FOR_TEST}\"' failed with "
                      "status 1 in inner (bash gives no line), called from probe.sh:7 in outer, "
                      "called from probe.sh:9 in main\n", result.stderr)

    def test_a_deliberate_exit_is_not_reported_again(self):
        for body, status in (("fct_die 'named by the script'\n", 1), ("exit 2\n", 2)):
            with self.subTest(body=body):
                result = self.run_bash(body)
                self.assertEqual(result.returncode, status)
                self.assertNotIn("stopped unexpectedly", result.stderr)

    def test_handled_failures_report_nothing(self):
        result = self.run_bash("a=$(false; true)\nb=$(false) || true\nif ! c=$(exit 3); then :; fi\n"
                               "f() { return 4; }\nf || true\n")
        self.assertEqual((result.returncode, result.stderr), (0, ""))

    def test_a_script_exit_hook_runs_after_the_report(self):
        result = self.run_bash("fct_on_exit() { printf 'hook %s\\n' \"${1}\" >&2; }\nfalse\n")
        self.assertEqual(result.returncode, 1)
        self.assertRegex(result.stderr, r"^ERROR: probe\.sh stopped unexpectedly \(exit 1\): 'false' failed "
                                        r"with status 1 at probe\.sh:4 in main\nhook 1\n$")


class ProxyKeepAliveTests(unittest.TestCase):
    """Caddy must drop an idle upstream connection before core does, or a
    request (a POST is not retried) can land on one core is closing: 502."""

    def core_idle_seconds(self):
        serve = (REPO / "crates/api/src/serve.rs").read_text()
        match = re.search(r"idle: Duration::from_secs\((\d+)\)", serve)
        self.assertIsNotNone(match, "core's idle limit not found")
        return int(match.group(1))

    def test_both_example_sites_keep_upstream_connections_shorter_than_core(self):
        idle = self.core_idle_seconds()
        for name in ("Caddyfile.example", "Caddyfile.cutover.example"):
            with self.subTest(site=name):
                text = (REPO / "packaging" / name).read_text()
                proxy = text[text.index("reverse_proxy 127.0.0.1:8003"):]
                match = re.search(r"transport http \{\s*keepalive (\d+)s\s*\}", proxy)
                self.assertIsNotNone(match, f"{name}: no upstream keepalive")
                self.assertLess(int(match.group(1)), idle)


if __name__ == "__main__":
    unittest.main()
