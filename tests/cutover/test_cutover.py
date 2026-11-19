"""Stub-driven tests for scripts/cutover/{preflight,activate,return,measure}.sh.

No real systemctl, docker, ss, free, curl, dpkg or caddy is ever run: PATH points
at copies of tests/cutover/stubs/stub.sh, and host paths live under a temp root.
Run: python3 -m unittest discover -s tests/cutover
"""

import hashlib
import os
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
    "caddy", "sleep", "limeosctl",
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
MUTATING_VERBS = {"stop", "start", "restart", "enable", "disable", "reload", "daemon-reload"}
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


def tree_digest(*roots, skip=("calls.log",)):
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

    def run_script(self, name, *args, now=NOW):
        env = {**self.env, "CUTOVER_NOW": str(now)}
        return subprocess.run(
            ["bash", str(SCRIPTS / name), *args], env=env, capture_output=True, text=True, timeout=60)

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

    def activate(self, *extra, now=NOW + 60, attached=True):
        args = self.activate_args(*extra, *(["--attached"] if attached else []))
        args[0] = str(self.snap)
        return self.run_script("activate.sh", *args, now=now)

    def approved(self, *extra, now=NOW + 60, attached=True):
        return self.activate("--yes-i-have-operator-approval", *extra, now=now, attached=attached)

    def do_return(self, *extra):
        return self.run_script("return.sh", str(self.snap), *extra)

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
                               (None, "Jellyfin playback could not be checked (HTTP 000)")):
            with self.subTest(code=code):
                self.setUp()
                self.set_curl({SESSIONS: code} if code else {})
                self.refused(fragment)

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

    def test_state_files_override_still_works(self):
        self.env["CUTOVER_STATE_FILES"] = "/etc/limeos/credentials.env"
        result = self.preflight(state_list=False)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertNotIn("/home/holly", (self.snap / "state.sha256").read_text())

    def test_refused_snapshot_blocks_activate(self):
        (self.stub / "free_out").write_text(FREE_OK.replace("6000000000", "1000000000"))
        self.preflight()
        result = self.approved()
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.mutating_calls(), [])


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
        for point in ACTIVATE_STEPS:
            for sig in ("INT", "TERM", "HUP"):
                with self.subTest(point=point, signal=sig):
                    self.setUp()
                    self.passing_snapshot()
                    (self.stub / "signal.txt").write_text(f"{sig} {point}\n")
                    result = self.approved()
                    self.assert_rolled_back(result)
                    self.assertIn(f"interrupted by SIG{sig}", result.stderr)
                    # The EXIT trap after the rollback must not run it again.
                    self.assertEqual(result.stderr.count("ROLLBACK: running return.sh"), 1)
                    self.assertNotIn("ROLLBACK INCOMPLETE", result.stderr)

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

    @staticmethod
    def wait_for(predicate, timeout=30):
        deadline = time.monotonic() + timeout
        while not predicate():
            if time.monotonic() > deadline:
                raise AssertionError("timed out")
            time.sleep(0.05)

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
        self.assertTrue([l for l in self.calls() if l.startswith("systemd-run --scope ")], self.calls())
        # The run had its own session, so the terminal's hangup cannot reach it.
        self.assertNotEqual(int(reached.read_text()), os.getsid(0))
        for name in RUST:
            self.assertEqual(self.unit_state(name), ("active", "enabled"))

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

    def test_idempotent(self):
        self.activated()
        self.assertEqual(self.do_return().returncode, 0)
        digest = tree_digest(self.root), tree_digest(self.stub / "units")
        second = self.do_return()
        self.assertEqual(second.returncode, 0, second.stderr)
        self.assertEqual(digest, (tree_digest(self.root), tree_digest(self.stub / "units")))

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


if __name__ == "__main__":
    unittest.main()
