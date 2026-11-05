#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Refusal tests for local cross-build provenance. No SSH, guests or ARM64 execution."""

import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

import cross_build
import qualification as q

HERE = Path(__file__).resolve().parent
SOURCE = "a" * 40


def record():
    return {
        "exit_code": 0,
        "source": SOURCE,
        "target": "aarch64-unknown-linux-gnu",
        "offline": True,
        "locked": True,
        "image": "sha256:" + "1" * 64,
        "binaries": {n: {"sha256": "2" * 64} for n in cross_build.PACKAGED},
    }


class RecordTests(unittest.TestCase):
    def test_accepts_a_complete_record(self):
        cross_build.check_record(record(), SOURCE)

    def test_refuses_each_weakening(self):
        for change in (
            {"exit_code": 1},
            {"source": "b" * 40},
            {"target": "x86_64-unknown-linux-gnu"},
            {"offline": False},
            {"locked": False},
            {"image": None},
            {"binaries": {}},
        ):
            with self.assertRaises(ValueError, msg=str(change)):
                cross_build.check_record({**record(), **change}, SOURCE)

    def test_stack_read_worker_is_packaged_and_checked(self):
        self.assertIn("limeos-stack-read-worker", cross_build.PACKAGED)
        self.assertTrue(set(q.BINARIES) < set(cross_build.PACKAGED))


class BuildKindTests(unittest.TestCase):
    def test_cross_build_requires_its_adjacent_inputs(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "build-result.json"
            build = {
                "build_kind": cross_build.BUILD_KIND,
                "provenance_version": 1,
                "identity": {
                    "source_commit": SOURCE,
                    "fixtures_commit": "b" * 40,
                    "package_version": "0.4.16+arm64.1",
                    "architecture": "arm64",
                    "authority_schema": 15,
                },
                "input_sha256": {"source.tar.gz": "0" * 64},
                "repository_sha256": {},
            }
            path.write_text(json.dumps(build))
            with self.assertRaises(ValueError):
                q.load_build(path)

    def test_native_gates_do_not_admit_a_cross_build(self):
        with self.assertRaises(ValueError):
            q.validate_native_gates(
                {
                    "build_kind": cross_build.BUILD_KIND,
                    "cross_build": {},
                    "steps": [],
                    "ci": 1,
                }
            )

    def test_unknown_kind_is_refused(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "b.json"
            identity = {
                "source_commit": SOURCE,
                "fixtures_commit": "b" * 40,
                "package_version": "1.0",
                "architecture": "arm64",
                "authority_schema": 15,
            }
            path.write_text(
                json.dumps({"identity": identity, "build_kind": "cross-ish"})
            )
            with self.assertRaisesRegex(ValueError, "unknown build provenance kind"):
                q.load_build(path)


class StackGuestTests(unittest.TestCase):
    def test_refuses_outside_a_disposable_native_guest(self):
        done = subprocess.run(
            [
                sys.executable,
                str(HERE / "stack_guest.py"),
                "--suite",
                "scale",
                "--repo",
                "/nonexistent",
                "--expected",
                "/nonexistent",
                "--output",
                "/nonexistent/out.json",
            ],
            capture_output=True,
            text=True,
            check=False,
        )
        self.assertNotEqual(done.returncode, 0)
        self.assertIn("disposable limeos-p01-test guest required", done.stderr)

    def test_wrapper_runs_only_the_bundled_unchanged_fixtures(self):
        source = (HERE / "stack_guest.py").read_text()
        for needle in (
            "verify_files(fixtures",
            "p04_stack_lifecycle_guest",
            "p04_stack_scale_guest",
            "load_build(args.expected, args.repo)",
        ):
            self.assertIn(needle, source)


if __name__ == "__main__":
    unittest.main()
