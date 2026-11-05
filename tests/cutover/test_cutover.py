"""Stub-driven tests for scripts/cutover/{preflight,activate,return}.sh.

No real systemctl, docker, ss, free, curl, dpkg or caddy is ever run: PATH points
at copies of tests/cutover/stubs/stub.sh, and host paths live under a temp root.
Run: python3 -m unittest discover -s tests/cutover
"""

import hashlib
import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
SCRIPTS = REPO / "scripts/cutover"
STUB = Path(__file__).resolve().parent / "stubs/stub.sh"
TOOLS = [
    "systemctl", "docker", "ss", "free", "curl", "dpkg", "dpkg-query",
    "caddy", "sleep", "limeosctl",
]
NOW = 1_800_000_000
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
MUTATING_VERBS = {"stop", "start", "restart", "enable", "disable", "reload", "daemon-reload"}
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
        self._write("/etc/caddy/Caddyfile", "import /etc/caddy/conf.d/*.caddy\n")
        self._write("/pkg/limeos_1.0.0_arm64.deb", "deb")
        for name in [*FROZEN, *UNTOUCHED]:
            self._unit(name)
        self._unit("caddy.service", "inactive", "disabled")
        for name in RUST:
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
            "http://127.0.0.1:8096/Sessions": "401",
            f"{SITE}/api/v1/health": "200",
            f"{SITE}/": "200 <html>",
            "http://127.0.0.1:8002/": "200",
        })

    def set_curl(self, mapping):
        (self.stub / "curl.map").write_text("".join(f"{k} {v}\n" for k, v in mapping.items()))

    def inject(self, call):
        with open(self.stub / "fail.txt", "a") as handle:
            handle.write(call + "\n")

    def run_script(self, name, *args, now=NOW):
        env = {**self.env, "CUTOVER_NOW": str(now)}
        return subprocess.run(
            ["bash", str(SCRIPTS / name), *args], env=env, capture_output=True, text=True, timeout=60)

    def preflight(self, *extra):
        return self.run_script("preflight.sh", "--out", self.out, *extra)

    def activate_args(self, *extra):
        return [
            str(Path(self.out)), "--package", "/pkg/limeos_1.0.0_arm64.deb",
            "--expected-version", "1.0.0", "--host", HOST, *extra,
        ]

    def activate(self, *extra, now=NOW + 60):
        args = self.activate_args(*extra)
        args[0] = str(self.snap)
        return self.run_script("activate.sh", *args, now=now)

    def approved(self, *extra, now=NOW + 60):
        return self.activate("--yes-i-have-operator-approval", *extra, now=now)

    def do_return(self, *extra):
        return self.run_script("return.sh", str(self.snap), *extra)

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

    def test_refuses_stopped_managed_container(self):
        lines = (self.stub / "docker_ps.txt").read_text().splitlines()
        (self.stub / "docker_ps.txt").write_text("\n".join(l for l in lines if "radarr" not in l) + "\n")
        self.refused("managed container not running: radarr")

    def test_refuses_active_playback(self):
        self.set_curl({"http://127.0.0.1:8096/Sessions": '200 [{"NowPlayingItem":{}}]'})
        self.refused("active playback")

    def test_warns_when_playback_unchecked(self):
        result = self.preflight()
        self.assertEqual(result.returncode, 0)
        self.assertIn("could not be checked", result.stderr)

    def test_idle_jellyfin_without_credentials_passes(self):
        self.set_curl({"http://127.0.0.1:8096/Sessions": "200 []"})
        self.assertEqual(self.preflight().returncode, 0)

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
        self.assertIn("[dry-run] dpkg -i", result.stdout)
        self.assertEqual(self.mutating_calls(), [])
        self.assertEqual(before, (tree_digest(self.root), tree_digest(self.stub)))

    def test_happy_path_order_and_scope(self):
        self.passing_snapshot()
        result = self.approved()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        mutating = self.mutating_calls()
        expected = [
            "systemctl stop limeos-metrics-collector.timer limeos-package-reconcile.timer",
            "systemctl stop pi-health.service",
            "systemctl stop pihealth-helper.service",
            "systemctl disable pihealth-helper.service",
            "systemctl disable pi-health.service",
            "systemctl disable limeos-metrics-collector.timer",
            "systemctl disable limeos-package-reconcile.timer",
            "dpkg -i /pkg/limeos_1.0.0_arm64.deb",
            "systemctl daemon-reload",
            "systemctl enable limeos-storaged.service",
            "systemctl enable limeos-containerd.service",
            "systemctl enable limeos-core.service",
            "systemctl start limeos-storaged.service",
            "systemctl start limeos-containerd.service",
            "systemctl start limeos-core.service",
            "caddy validate --adapter caddyfile --config /etc/caddy/Caddyfile",
            "systemctl enable caddy.service",
            "systemctl start caddy.service",
        ]
        self.assertEqual(mutating, expected)
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
            "dpkg -i /pkg/limeos_1.0.0_arm64.deb",
            "systemctl daemon-reload",
            "systemctl enable limeos-core.service",
            "systemctl start limeos-containerd.service",
            "systemctl start limeos-core.service",
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

    def test_version_mismatch_rolls_back(self):
        self.passing_snapshot()
        (self.stub / "pkg_to_install").write_text("9.9.9\n")
        self.assert_rolled_back(self.approved())

    def test_container_change_rolls_back(self):
        self.passing_snapshot()
        snap_ps = self.snap / "docker-ps.txt"
        snap_ps.write_text(snap_ps.read_text().replace("0001", "ffff", 1))
        self.assert_rolled_back(self.approved())

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

    def test_idempotent(self):
        self.activated()
        self.assertEqual(self.do_return().returncode, 0)
        digest = tree_digest(self.root), tree_digest(self.stub / "units")
        second = self.do_return()
        self.assertEqual(second.returncode, 0, second.stderr)
        self.assertEqual(digest, (tree_digest(self.root), tree_digest(self.stub / "units")))

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

    def test_live_database_difference_only_warns_without_baseline(self):
        self.passing_snapshot()
        self._write("/var/lib/limeos/metrics.db", "writer-ran-before-stop")
        result = self.do_return()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("live state file differs", result.stderr)


if __name__ == "__main__":
    unittest.main()
