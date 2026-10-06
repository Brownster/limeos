#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Local harness regression tests. No SSH, guests or installed-service mutations."""

import copy
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import qualification as q
import record_run
from native_vm import Guest
from record_run import budget_table

HERE = Path(__file__).resolve().parent


def build_fixture():
    identity = {
        "source_commit": "a" * 40,
        "fixtures_commit": "b" * 40,
        "package_version": "0.4.4+arm64.1",
        "architecture": "arm64",
        "authority_schema": 8,
    }
    return {
        "identity": identity,
        "steps": [
            {"step": step, "exit": 0}
            for step in (
                "apt",
                "rustup",
                "fetch",
                "fmt",
                "clippy",
                "test",
                "contracts",
                "dependency-direction",
                "release",
                "package-standard",
                "package-shadow",
                "signing-key",
                "repository",
            )
        ],
        "binaries": {n: {"sha256": "c" * 64, "machine": "AArch64"} for n in q.BINARIES},
        "packages": {
            f"{n}_0.4.4+arm64.1_arm64.deb": "d" * 64
            for n in ("limeos", "limeos-shadow")
        },
        "tests": {"passed": 185, "failed": 0},
        "package_control": {
            f"{n}_0.4.4+arm64.1_arm64.deb": f"Package: {n}\nVersion: 0.4.4+arm64.1\nArchitecture: arm64\n"
            for n in ("limeos", "limeos-shadow")
        },
    }


class IdentityTests(unittest.TestCase):
    def test_rejects_mixed_package_architecture_failed_gate_and_incomplete_build(self):
        original = build_fixture()
        mutations = []
        failed = copy.deepcopy(original)
        failed["steps"][0]["exit"] = 1
        mutations.append(failed)
        missing = copy.deepcopy(original)
        missing["steps"].pop()
        mutations.append(missing)
        relabeled = copy.deepcopy(original)
        relabeled["identity"]["package_version"] = "0.4.2"
        mutations.append(relabeled)
        x86 = copy.deepcopy(original)
        x86["binaries"]["limeos-core"]["machine"] = "X86-64"
        mutations.append(x86)
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "build.json"
            path.write_text(json.dumps(original))
            self.assertEqual(q.load_build(path), original)
            for mutation in mutations:
                with self.subTest(mutation=mutation):
                    path.write_text(json.dumps(mutation))
                    with self.assertRaises(ValueError):
                        q.load_build(path)

    def test_file_hash_drift_and_escape_are_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "source.rs").write_text("frozen")
            recorded = q.sha(root / "source.rs")
            self.assertEqual(q.verify_files(root, {"source.rs": recorded}), 1)
            (root / "source.rs").write_text("edited")
            with self.assertRaises(ValueError):
                q.verify_files(root, {"source.rs": recorded})
            with self.assertRaises(ValueError):
                q.verify_files(root, {"../outside": recorded})
            (root / "alias").symlink_to(root / "source.rs")
            with self.assertRaises(ValueError):
                q.verify_files(root, {"alias": q.sha(root / "source.rs")})

    def test_source_schema_must_match_manifest(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            file = root / "source/crates/persistence/src/lib.rs"
            file.parent.mkdir(parents=True)
            file.write_text("pub const SCHEMA_VERSION: u32 = 8;")
            (root / "source/frontend").mkdir()
            (root / "source/frontend/index.html").write_text("UI")
            (root / "fixtures").mkdir()
            (root / "fixtures/test.py").write_text("fixture")
            manifest = {
                "identity": {"authority_schema": 6},
                "runtime_source_sha256": {"crates/persistence/src/lib.rs": q.sha(file)},
                "frontend_dist_sha256": {
                    "frontend/index.html": q.sha(root / "source/frontend/index.html")
                },
                "fixtures_sha256": {"test.py": q.sha(root / "fixtures/test.py")},
            }
            with self.assertRaises(ValueError):
                q.verify_source(root, manifest)

    def test_disposable_marker_does_not_allow_emulation(self):
        with (
            patch.object(q.os, "geteuid", return_value=0),
            patch.object(q.socket, "gethostname", return_value="limeos-p01-test"),
            patch.object(q.platform, "machine", return_value="aarch64"),
            patch.object(q.subprocess, "check_output", return_value="qemu\n"),
            self.assertRaisesRegex(RuntimeError, "native KVM"),
        ):
            q.guest_guard()

    def test_guest_state_cannot_be_used_on_a_different_host(self):
        with (
            tempfile.TemporaryDirectory() as directory,
            patch("native_vm.STATE", Path(directory)),
        ):
            local = Path(directory) / "run"
            local.mkdir()
            (local / "guest.json").write_text(
                json.dumps({"host": "test-a", "port": 22801})
            )
            with self.assertRaises(ValueError):
                Guest("run", "test-b")
            for name in ("../escape", "x;touch file", "-bad"):
                with self.assertRaises(ValueError):
                    Guest(name, "test-a")


class EvidenceTests(unittest.TestCase):
    def test_evidence_cannot_overwrite_history_or_bound_build_artifact(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            manifest = {
                "identity": build_fixture()["identity"],
                "runtime_source_sha256": {"a": "a" * 64},
                "frontend_dist_sha256": {"b": "b" * 64},
                "fixtures_sha256": {"c": "c" * 64},
            }
            source = root / "source.json"
            source.write_text(json.dumps(manifest))
            build = build_fixture()
            build.update(
                {
                    k: manifest[k]
                    for k in (
                        "runtime_source_sha256",
                        "frontend_dist_sha256",
                        "fixtures_sha256",
                    )
                }
            )
            build["source_manifest_sha256"] = q.sha(source)
            result = root / "build.json"
            result.write_text(json.dumps(build))
            run = root / "docs/rewrite-evidence/arm64/run"
            arguments = [
                "record_run.py",
                "--run",
                "run",
                "--source-manifest",
                str(source),
                "--build-result",
                str(result),
            ]
            with (
                patch.object(record_run, "ROOT", root),
                patch.object(
                    sys, "argv", [*arguments, "--artifact", str(root) + "=build"]
                ),
                self.assertRaises(SystemExit) as caught,
            ):
                record_run.main()
            self.assertEqual(caught.exception.code, 2)
            self.assertFalse(run.exists())
            run.mkdir(parents=True)
            sentinel = run / "historical.json"
            sentinel.write_text("historical bytes")
            with (
                patch.object(record_run, "ROOT", root),
                patch.object(sys, "argv", arguments),
                self.assertRaises(SystemExit) as caught,
            ):
                record_run.main()
            self.assertEqual(caught.exception.code, 2)
            self.assertEqual(sentinel.read_text(), "historical bytes")

    def test_missing_write_counters_are_unavailable_and_short_idle_is_not_qualified(
        self,
    ):
        window = {
            "seconds": 600,
            "limeos_cpu_percent_of_one_core": 0.01,
            "per_unit": {
                "limeos-core": {"process_write_bytes": 4096},
                "limeos-storaged": {},
            },
        }
        install = {
            "checks": [
                {
                    "name": "idle CPU and bytes written over 600 seconds, no subscribers",
                    "result": "pass",
                    "observation": window,
                }
            ]
        }
        rows = budget_table(install)
        self.assertEqual(rows[-1][1], "unavailable")
        self.assertIsNone(rows[-1][-1])
        window["seconds"] = 30
        self.assertEqual(budget_table(install), [])

    def test_all_entry_points_offer_help_without_guest_access(self):
        for name in (
            "build_guest",
            "install_guest",
            "upgrade_guest",
            "extra_guest",
            "approved_guest",
            "native_vm",
            "make_bundle",
            "record_run",
        ):
            with self.subTest(script=name):
                result = subprocess.run(
                    [sys.executable, str(HERE / (name + ".py")), "--help"],
                    capture_output=True,
                    text=True,
                    timeout=10,
                    check=False,
                )
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertIn("usage:", result.stdout)


if __name__ == "__main__":
    unittest.main()
