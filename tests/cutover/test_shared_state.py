"""The first wybie attempt (26 Nov 2026), modelled against the stubs.

pi-health.service and limeos-metrics-collector.service declare StateDirectory=limeos, so systemd
owns /var/lib/limeos on their behalf (holly:pihealth 0750) and chowns it, LimeOS's trees inside
included, at every start whose owner check fails; the limeops-* services keep writing their homes
under /var/lib/limeos while the cutover runs; the scripts were copied to the host as loose files.
Run: python3 -m unittest discover -s tests/cutover
"""

import json
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

from test_cutover import HOST, REPO, RUST, SCRIPTS, Harness

CLAIMANTS = ["pi-health.service", "limeos-metrics-collector.service"]
ACCOUNTS = {"limeos-core": 990, "limeos-assistant": 991, "limeos-containerd": 992}


class SharedStateHarness(Harness):
    def claimed(self):
        """wybie's units: the app and the metrics collector own /var/lib/limeos; LimeOS's accounts exist."""
        for unit in CLAIMANTS:
            (self.stub / "units" / f"{unit}.statedir").write_text("limeos\n")
        (self.stub / "passwd").write_text(
            "root:x:0:0:root:/root:/bin/bash\n"
            + "".join(f"{name}:x:{uid}:{uid}::/nonexistent:/usr/sbin/nologin\n" for name, uid in ACCOUNTS.items()))
        # LimeOS's trees as the old app's start left them (owned by the test user here, never 990-992).
        self._write("/var/lib/limeos/core/core.sqlite", "core-db")
        self._write("/var/lib/limeos/executors/container/receipts.sqlite", "receipts")
        self._write("/var/lib/limeos/assistant/state.json", "{}")

    def guard(self, unit):
        return self.root / f"etc/systemd/system/{unit}.d/zz-limeos-cutover-guard.conf"


class PreflightSharedStateTests(SharedStateHarness):
    def test_records_who_owns_the_state_directory_and_the_shared_directories(self):
        self.claimed()
        result = self.preflight()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual((self.snap / "state-claimants.txt").read_text().split(), CLAIMANTS)
        records = (self.snap / "shared-dirs.txt").read_text().splitlines()
        self.assertEqual([r.split()[0] for r in records], ["/etc/limeos", "/var/lib/limeos"])
        st = (self.root / "var/lib/limeos").stat()
        self.assertIn(f"/var/lib/limeos {st.st_uid} {st.st_gid} {oct(st.st_mode & 0o777)[2:]}", records)
        self.assertEqual(self.mutating_calls(), [])

    def test_a_host_without_claimants_records_none(self):
        result = self.preflight()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual((self.snap / "state-claimants.txt").read_text(), "")

    def test_an_unanswered_state_directory_query_is_refused(self):
        self.silent("systemctl show -p StateDirectory --value pi-health.service")
        result = self.preflight()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("could not determine which frozen services declare StateDirectory=limeos", result.stderr)

    def test_warns_when_limeos_runs_while_the_old_app_owns_the_directory(self):
        # wybie at 01:06: the package's configure had started the Rust services.
        self.claimed()
        for name in RUST:
            self._unit(name, "active", "enabled")
        result = self.preflight()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("LimeOS services are running while pi-health.service limeos-metrics-collector.service own /var/lib/limeos", result.stderr)
        self.assertIn("systemctl disable --now limeos-storaged.service limeos-containerd.service limeos-core.service", result.stderr)

    def test_a_guard_left_by_an_earlier_activation_is_refused(self):
        self._write("/etc/systemd/system/pi-health.service.d/zz-limeos-cutover-guard.conf", "[Unit]\n")
        result = self.preflight()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("a cutover guard from an earlier activation is still in place: "
                      "/etc/systemd/system/pi-health.service.d/zz-limeos-cutover-guard.conf", result.stderr)


class LiveStateTests(SharedStateHarness):
    """State the limeops-* services and outside schedules write is reported, never failed on."""

    def setUp(self):
        super().setUp()
        self._write("/var/lib/limeos/agent-actions/supervision.sqlite3", "sup-1")
        self._write("/var/lib/limeos/agent-actions/supervision.sqlite3-journal", "j")
        self._write("/var/lib/limeos/agent-supervisor/venv/pyvenv.cfg", "home = /usr/bin\n")
        self._write("/var/lib/limeos/backup_config.json", '{"b":1}')
        self.live_list = Path(self._tmp.name) / "live-paths.txt"
        self.live_list.write_text(
            "# home of the limeops-* services, which the cutover leaves running\n"
            "/var/lib/limeos/agent-actions/\n/var/lib/limeos/agent-supervisor/\n"
            "# rewritten at 04:00 and 02:00 by schedules outside the frozen app\n"
            "/var/lib/limeos/auto_update.json\n/var/lib/limeos/backup_config.json\n")

    def live_preflight(self):
        result = self.preflight("--live-list", str(self.live_list))
        self.assertEqual(result.returncode, 0, result.stderr)
        (self.stub / "calls.log").write_text("")
        return result

    def test_live_paths_are_recorded_as_such_and_copied_into_the_snapshot(self):
        self.live_preflight()
        records = {line.split(" ", 2)[2]: line.split(" ")[0] for line in (self.snap / "state.sha256").read_text().splitlines()}
        for path in ("/var/lib/limeos/agent-actions/supervision.sqlite3", "/var/lib/limeos/agent-actions/supervision.sqlite3-journal",
                     "/var/lib/limeos/agent-supervisor/venv/pyvenv.cfg", "/var/lib/limeos/auto_update.json",
                     "/var/lib/limeos/backup_config.json"):
            self.assertEqual(records[path], "L", path)
        self.assertEqual(records["/etc/limeos/credentials.env"], "S")
        self.assertEqual(records["/var/lib/limeos/metrics.db"], "V")
        self.assertEqual((self.snap / "live-paths.txt").read_text(), self.live_list.read_text())

    def test_a_return_spanning_their_writes_is_verified_with_warnings(self):
        # 26 Nov: supervision.sqlite3 changed after the post-stop baseline and the rollback said
        # ROLLBACK INCOMPLETE. The two JSON files stand for files an operator has established an outside
        # writer for (on wybie that is not established for auto_update.json or backup_config.json: they stay strict).
        self.live_preflight()
        result = self.approved()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self._write("/var/lib/limeos/agent-actions/supervision.sqlite3", "sup-2")
        (self.root / "var/lib/limeos/agent-actions/supervision.sqlite3-journal").unlink()
        self._write("/var/lib/limeos/auto_update.json", '{"rewritten":"04:00"}')
        self._write("/var/lib/limeos/backup_config.json", '{"rewritten":"02:00"}')
        result = self.do_return()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("return verified", result.stdout)
        self.assertIn("WARN: live state changed since preflight (not a failure: home of the limeops-* services, which the cutover "
                      "leaves running): /var/lib/limeos/agent-actions/supervision.sqlite3", result.stderr)
        self.assertIn("(not a failure: rewritten at 04:00 and 02:00 by schedules outside the frozen app): /var/lib/limeos/auto_update.json",
                      result.stderr)
        self.assertIn("4 changed (reported only)", result.stdout)

    def test_the_frozen_apps_own_state_still_fails(self):
        self.live_preflight()
        self.assertEqual(self.approved().returncode, 0)
        self._write("/var/lib/limeos/agent-actions/supervision.sqlite3", "sup-2")
        self._write("/home/holly/pi-health/config/backup_config.json", '{"edited":2}')
        self._write("/var/lib/limeos/metrics.db", "late-writer")
        result = self.do_return()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("FAIL: state file changed: /home/holly/pi-health/config/backup_config.json", result.stderr)
        self.assertIn("FAIL: live state file changed after the writers stopped: /var/lib/limeos/metrics.db", result.stderr)
        self.assertNotIn("FAIL: live state file changed after the writers stopped: /var/lib/limeos/agent-actions", result.stderr)

    def test_without_a_live_list_the_supervision_writer_still_fails_the_return(self):
        # What happened on 26 Nov, kept as the behaviour for an unlisted writer.
        self.passing_snapshot()
        self.assertEqual(self.approved().returncode, 0)
        self._write("/var/lib/limeos/agent-actions/supervision.sqlite3", "sup-2")
        result = self.do_return()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("live state file changed after the writers stopped: /var/lib/limeos/agent-actions/supervision.sqlite3", result.stderr)

    def test_a_live_list_cannot_exempt_the_frozen_app(self):
        cases = {
            "/var/lib/limeos/\n": "would exempt a whole shared directory",
            "/etc/limeos\n": "would exempt a whole shared directory",
            "/var/lib/limeos/nothing-here.json\n": "live list path missing: /var/lib/limeos/nothing-here.json",
            "/var/lib/limeos/agent-actions\n/var/lib/limeos/no-such-dir/\n": "live list directory missing (or not a directory): /var/lib/limeos/no-such-dir/",
            "relative/path\n": "live list entry is not an absolute path: relative/path",
            "/home/holly/pi-health/config/\n": "state list path /home/holly/pi-health/config/backup_config.json is also covered by the live list",
        }
        for text, fragment in cases.items():
            with self.subTest(text=text):
                self.live_list.write_text(text)
                shutil.rmtree(self.snap, ignore_errors=True)
                result = self.preflight("--live-list", str(self.live_list))
                self.assertNotEqual(result.returncode, 0, result.stderr)
                self.assertIn(fragment, result.stderr)


class ActivateSharedStateTests(SharedStateHarness):
    def test_takes_the_state_directory_over_before_the_package_and_guards_the_old_units(self):
        self.claimed()
        self.passing_snapshot()
        result = self.approved()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        calls = self.calls()
        root = str(self.root)
        disable = calls.index("systemctl disable limeos-package-reconcile.timer")
        dpkg = calls.index("dpkg --force-confold -i /pkg/limeos_1.0.0_arm64.deb")
        takeover = [c for c in calls[disable + 1:dpkg] if c.split()[0] in ("systemctl", "chown", "chmod")]
        self.assertEqual(takeover[:3], ["systemctl daemon-reload", f"chown root:root -- {root}/var/lib/limeos", f"chmod 0755 -- {root}/var/lib/limeos"])
        self.assertIn(f"chown -h root:root -- {root}/var/lib/limeos/executors", takeover)
        # Every entry of LimeOS's trees goes back to its account and primary group.
        for tree, account in (("core", "limeos-core"), ("executors/container", "limeos-containerd"), ("assistant", "limeos-assistant")):
            uid = ACCOUNTS[account]
            self.assertTrue(any(c.startswith(f"chown -h {uid}:{uid} -- ") and f"{root}/var/lib/limeos/{tree}" in c for c in takeover), (tree, takeover))
        # Nothing of the frozen app's is changed.
        self.assertFalse([c for c in calls if c.startswith("chown") and ("credentials" in c or "metrics.db" in c or "pi-health" in c)])
        for unit in CLAIMANTS:
            text = self.guard(unit).read_text()
            self.assertIn(f"AssertPathExists=!/etc/systemd/system/{unit}.d/zz-limeos-cutover-guard.conf", text)
            self.assertIn("return.sh removes this file", text)
        self.assertFalse(self.guard("pihealth-helper.service").exists())

    def test_without_claimants_nothing_is_taken_over(self):
        self.passing_snapshot()
        self.assertEqual(self.approved().returncode, 0)
        self.assertFalse([c for c in self.calls() if c.split()[0] in ("chown", "chmod")])
        self.assertFalse(list((self.root / "etc/systemd/system").glob("*.d/zz-limeos-cutover-guard.conf")) if (self.root / "etc/systemd/system").exists() else [])

    def test_a_dry_run_prints_the_takeover_and_changes_nothing(self):
        self.claimed()
        self.passing_snapshot()
        result = self.activate("--dry-run")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn(f"[dry-run] chown root:root -- {self.root}/var/lib/limeos", result.stdout)
        self.assertIn("[dry-run] install -m 0644 <guard>", result.stdout)
        self.assertFalse(self.guard("pi-health.service").exists())
        self.assertEqual(self.mutating_calls(), [])

    def test_a_snapshot_without_the_shared_state_records_is_refused(self):
        self.passing_snapshot()
        (self.snap / "state-claimants.txt").unlink()
        result = self.approved()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("snapshot has no state-claimants.txt or shared-dirs.txt", result.stderr)
        self.assertEqual(self.mutating_calls(), [])

    def test_a_failed_takeover_rolls_back(self):
        self.claimed()
        self.passing_snapshot()
        self.inject(f"chown root:root -- {self.root}/var/lib/limeos")
        result = self.approved()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("ROLLBACK: running return.sh", result.stderr)
        self.assertFalse(self.guard("pi-health.service").exists())
        self.assertEqual(self.unit_state("pi-health.service"), ("active", "enabled"))


class ReturnSharedStateTests(SharedStateHarness):
    def activated_claimed(self, rust_before=("inactive", "disabled")):
        self.claimed()
        for name in RUST:
            self._unit(name, *rust_before)
        self.passing_snapshot()
        result = self.approved()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        (self.stub / "calls.log").write_text("")

    def test_gives_the_directory_back_before_the_old_app_starts_and_leaves_limeos_stopped(self):
        # 26 Nov: the rollback restarted pi-health with /var/lib/limeos root's; systemd chowned
        # everything to holly, core lost its database, and return.sh said nothing about it.
        self.activated_claimed(rust_before=("active", "enabled"))
        result = self.do_return()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        calls = self.calls()
        first_frozen_start = calls.index("systemctl start pihealth-helper.service")
        for unit in CLAIMANTS:
            self.assertFalse(self.guard(unit).exists(), unit)
        before = calls[:first_frozen_start]
        self.assertIn("systemctl daemon-reload", before)
        for name in RUST:
            self.assertIn(f"systemctl stop {name}", before)
            self.assertIn(f"systemctl disable {name}", before)
            self.assertNotIn(f"systemctl start {name}", calls)
            self.assertEqual(self.unit_state(name), ("inactive", "disabled"), name)
        self.assertIn("leaving the LimeOS services stopped and disabled: /var/lib/limeos belongs to "
                      "pi-health.service limeos-metrics-collector.service again", result.stdout)
        # The directory's record matches already here (the stub chown changed nothing), so no chown is needed.
        self.assertFalse([c for c in calls if c.startswith("chown")], calls)
        self.assertEqual(self.unit_state("pi-health.service"), ("active", "enabled"))

    def test_restores_the_recorded_owner_when_it_differs(self):
        self.activated_claimed()
        # Recorded as holly:pihealth 0750 (uid 1000, gid 1001); the fake root's directory is the test user's.
        lines = [("/var/lib/limeos 1000 1001 750" if line.startswith("/var/lib/limeos ") else line)
                 for line in (self.snap / "shared-dirs.txt").read_text().splitlines()]
        (self.snap / "shared-dirs.txt").write_text("\n".join(lines) + "\n")
        result = self.do_return()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        calls = self.calls()
        start = calls.index("systemctl start pi-health.service")
        chown = calls.index(f"chown 1000:1001 -- {self.root}/var/lib/limeos")
        chmod = calls.index(f"chmod 750 -- {self.root}/var/lib/limeos")
        self.assertLess(max(chown, chmod), start)
        self.assertIn("giving /var/lib/limeos back to pi-health.service limeos-metrics-collector.service as preflight recorded it (1000:1001 750", result.stdout)

    def test_the_automatic_rollback_does_the_same(self):
        self.claimed()
        for name in RUST:
            self._unit(name, "active", "enabled")
        self.passing_snapshot()
        (self.stub / "reader-read-container-health").write_text('{"read":"container_health","result":"failed","reason":"the read failed: Unavailable"}\n')
        (self.stub / "reader-read-container-health.rc").write_text("1\n")
        result = self.approved()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("ROLLBACK: running return.sh", result.stderr)
        self.assertIn("return verified", result.stderr)
        self.assertNotIn("ROLLBACK INCOMPLETE", result.stderr)
        for unit in CLAIMANTS:
            self.assertFalse(self.guard(unit).exists())
        for name in RUST:
            self.assertEqual(self.unit_state(name), ("inactive", "disabled"), name)

    def test_without_claimants_restored_rust_units_must_still_work(self):
        for name in RUST:
            self._unit(name, "active", "enabled")
        self.passing_snapshot()
        self.assertEqual(self.approved().returncode, 0)
        self.inject("limeosctl --socket /run/limeos-core/core.sock status")
        result = self.do_return()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("the LimeOS services the snapshot records running were started again but do not work now that the frozen app runs: "
                      "limeosctl status exit 1", result.stderr)
        self.assertEqual(self.unit_state("pi-health.service"), ("active", "enabled"))

    def test_unknown_claimants_fall_back_to_the_snapshot(self):
        self.activated_claimed(rust_before=("active", "enabled"))
        self.silent("systemctl show -p StateDirectory --value pi-health.service")
        result = self.do_return()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("using the snapshot's record", result.stderr)
        for name in RUST:
            self.assertEqual(self.unit_state(name), ("inactive", "disabled"), name)


class LayoutTests(SharedStateHarness):
    """26 Nov: the scripts copied to /root as loose files could not find the Caddy site template."""

    def bundle(self):
        out = Path(self._tmp.name) / "limeos-cutover"
        result = subprocess.run(["bash", str(SCRIPTS / "bundle.sh"), str(out)], capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        return out

    def run_from(self, directory, name, *args):
        return subprocess.run(["bash", str(directory / name), *args], env=self.env, capture_output=True, text=True, timeout=60)

    def test_bundle_holds_everything_with_checksums(self):
        out = self.bundle()
        names = sorted(p.name for p in out.iterdir())
        self.assertEqual(names, sorted(["lib.sh", "preflight.sh", "activate.sh", "return.sh", "measure.sh", "state_lists.py",
                                        "Caddyfile.cutover.example", "p06-cutover-runbook.md", "SHA256SUMS"]))
        check = subprocess.run(["sha256sum", "--strict", "-c", "SHA256SUMS"], cwd=out, capture_output=True, text=True)
        self.assertEqual(check.returncode, 0, check.stdout + check.stderr)
        again = subprocess.run(["bash", str(SCRIPTS / "bundle.sh"), str(out)], capture_output=True, text=True)
        self.assertNotEqual(again.returncode, 0)
        self.assertIn("exists and is not empty", again.stderr)

    def test_preflight_and_a_dry_run_work_from_a_copied_directory(self):
        out = self.bundle()
        result = self.run_from(out, "preflight.sh", "--out", self.out, "--state-list", str(self.state_list),
                               "--jellyfin-key-file", str(self.key_file), "--host", HOST,
                               "--package", "/pkg/limeos_1.0.0_arm64.deb", "--expected-version", "1.0.0")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertNotIn("activation inputs not checked", result.stderr)
        dry = self.run_from(out, "activate.sh", str(self.snap), "--dry-run", "--package", "/pkg/limeos_1.0.0_arm64.deb",
                            "--expected-version", "1.0.0", "--host", HOST)
        self.assertEqual(dry.returncode, 0, dry.stderr)
        self.assertIn(f"[dry-run] render {out}/Caddyfile.cutover.example", dry.stdout)

    def test_preflight_refuses_what_activation_would_fail_on(self):
        out = self.bundle()
        # Loose scripts without the template, as copied on 26 Nov (no SHA256SUMS either).
        (out / "Caddyfile.cutover.example").unlink()
        (out / "SHA256SUMS").unlink()
        result = self.run_from(out, "preflight.sh", "--out", self.out, "--state-list", str(self.state_list),
                               "--jellyfin-key-file", str(self.key_file))
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(f"REFUSED: site template not readable: {out}/Caddyfile.cutover.example", result.stderr)
        self.assertIn("--site-template FILE", result.stderr)
        self.assertIn("activation inputs not checked", result.stderr)
        # A documented --site-template makes it pass.
        template = Path(self._tmp.name) / "site.caddy"
        shutil.copy(REPO / "packaging/Caddyfile.cutover.example", template)
        shutil.rmtree(self.snap)
        result = self.run_from(out, "preflight.sh", "--out", self.out, "--state-list", str(self.state_list),
                               "--jellyfin-key-file", str(self.key_file), "--site-template", str(template))
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_a_tampered_bundle_and_wrong_inputs_are_refused(self):
        out = self.bundle()
        with open(out / "return.sh", "a") as handle:
            handle.write("# changed\n")
        self._write("/etc/caddy/Caddyfile", "# Debian's default site\n:80 {\n}\n")
        result = self.run_from(out, "preflight.sh", "--out", self.out, "--state-list", str(self.state_list),
                               "--jellyfin-key-file", str(self.key_file), "--host", "other.lan",
                               "--package", "/pkg/limeos_1.0.0_arm64.deb", "--expected-version", "2.0.0")
        self.assertNotEqual(result.returncode, 0)
        for fragment in (f"a file in {out} does not match its SHA256SUMS",
                         "/etc/caddy/Caddyfile must contain: import /etc/caddy/conf.d/*.caddy",
                         "core.json origin must be https://other.lan:8002",
                         "/pkg/limeos_1.0.0_arm64.deb is version 1.0.0, not 2.0.0"):
            self.assertIn(fragment, result.stderr)

    def test_a_running_caddy_is_warned_about(self):
        self._unit("caddy.service", "active", "enabled")
        result = self.preflight()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("caddy.service is running before the cutover (Debian's caddy package starts it on install)", result.stderr)


class StateListsTests(unittest.TestCase):
    """scripts/cutover/state_lists.py against a report shaped like wybie's."""

    def report(self, venv_as_entry=True):
        files = [
            {"location": "etc", "path": "credentials.env", "kind": "file"},
            {"location": "etc", "path": "agent-policy.json", "kind": "file"},
            {"location": "etc", "path": "core.json", "kind": "file"},
            {"location": "etc", "path": "system-policy/container.json", "kind": "file"},
            {"location": "etc", "path": "management-catalog.json", "kind": "file"},
            {"location": "state", "path": "metrics.sqlite3", "kind": "file"},
            {"location": "state", "path": "auto_update.json", "kind": "file"},
            {"location": "state", "path": "backup_config.json", "kind": "file"},
            {"location": "state", "path": "agent-actions/supervision.sqlite3", "kind": "file"},
            {"location": "state", "path": "agent-supervisor/.profile", "kind": "file"},
            {"location": "state", "path": "core/core.sqlite", "kind": "file"},
            {"location": "state", "path": "executors/container/receipts.sqlite3", "kind": "file"},
            {"location": "state", "path": "link", "kind": "symlink"},
            {"location": "stacks", "path": "media/compose.yaml", "kind": "file"},
            {"location": "checkout", "path": "backup_config.json", "kind": "file"},
            {"location": "checkout", "path": "schemas/storage-contract.schema.json", "kind": "file"},
        ]
        if venv_as_entry:
            files.append({"location": "state", "path": "agent-supervisor/venv", "kind": "virtualenv"})
        else:
            files += [{"location": "state", "path": "agent-supervisor/venv/pyvenv.cfg", "kind": "file"},
                      {"location": "state", "path": "agent-supervisor/venv/lib/python3.11/site-packages/pip/__init__.py", "kind": "file"}]
        return {"complete": True, "locations": [
            {"label": "etc", "path": "/etc/limeos"}, {"label": "state", "path": "/var/lib/limeos"},
            {"label": "stacks", "path": "/opt/stacks"}, {"label": "checkout", "path": "/home/holly/pi-health/config/"}],
            "files": files}

    def build(self, report, *live):
        with tempfile.TemporaryDirectory() as tmp:
            scan = Path(tmp) / "scan.json"
            scan.write_text(json.dumps(report))
            args = ["python3", "-I", str(SCRIPTS / "state_lists.py"), "--scan", str(scan),
                    "--state-out", f"{tmp}/state.txt", "--live-out", f"{tmp}/live.txt"]
            for entry in live:
                args += ["--live", entry]
            result = subprocess.run(args, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            return Path(f"{tmp}/state.txt").read_text().splitlines(), Path(f"{tmp}/live.txt").read_text(), result.stdout

    def test_the_state_list_is_the_frozen_apps_files_only(self):
        for venv_as_entry in (True, False):
            with self.subTest(venv_as_entry=venv_as_entry):
                state, live, out = self.build(
                    self.report(venv_as_entry),
                    "/var/lib/limeos/agent-actions/=home of the limeops-* services, which keep running",
                    "/var/lib/limeos/agent-supervisor/=home of the limeops-* services, which keep running",
                    "/var/lib/limeos/auto_update.json=rewritten daily at 04:00 outside the frozen app")
                self.assertEqual(state, sorted([
                    "/etc/limeos/agent-policy.json", "/etc/limeos/credentials.env", "/home/holly/pi-health/config/backup_config.json",
                    "/home/holly/pi-health/config/schemas/storage-contract.schema.json", "/var/lib/limeos/backup_config.json",
                    "/var/lib/limeos/metrics.sqlite3"]))
                self.assertEqual(live, "# home of the limeops-* services, which keep running\n/var/lib/limeos/agent-actions/\n"
                                       "# home of the limeops-* services, which keep running\n/var/lib/limeos/agent-supervisor/\n"
                                       "# rewritten daily at 04:00 outside the frozen app\n/var/lib/limeos/auto_update.json\n")
                self.assertIn("left out: 5 files of the LimeOS package", out)
                self.assertIn("left out: 1 Compose files under the stacks directory", out)

    def test_a_live_entry_needs_a_reason(self):
        with tempfile.TemporaryDirectory() as tmp:
            scan = Path(tmp) / "scan.json"
            scan.write_text(json.dumps(self.report()))
            result = subprocess.run(["python3", "-I", str(SCRIPTS / "state_lists.py"), "--scan", str(scan), "--state-out", f"{tmp}/s",
                                     "--live-out", f"{tmp}/l", "--live", "/var/lib/limeos/agent-actions/"], capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("--live needs ABSOLUTE_PATH=REASON", result.stderr)


if __name__ == "__main__":
    unittest.main()
