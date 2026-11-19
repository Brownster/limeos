#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Evidence bundle build, verify and restore checks on disposable trees."""

import gzip
import importlib.util
import io
import json
import os
import tarfile
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path, PurePosixPath

SPEC = importlib.util.spec_from_file_location(
    "evidence_bundles",
    Path(__file__).resolve().parents[2] / "scripts/evidence_bundles.py",
)
bundles = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(bundles)

RUN = "docs/rewrite-evidence/p09/2026-01-01-run"


def write(root, relative, data, executable=False):
    path = Path(root) / relative
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(data)
    path.chmod(0o755 if executable else 0o644)
    return path


def snapshot(root):
    base = Path(root)
    return {
        path.relative_to(base).as_posix(): (path.read_bytes(), os.access(path, os.X_OK))
        for path in sorted(base.rglob("*"))
        if path.is_file()
    }


def fixture(root):
    write(root, f"{RUN}/README.md", b"# Run\n\nSee [record](record.json.gz).\n")
    write(root, f"{RUN}/SHA256SUMS", b"0  record.json.gz\n")
    write(root, f"{RUN}/result.json", b'{"result": "pass"}\n')
    write(root, f"{RUN}/record.json.gz", gzip.compress(b"{}", mtime=0))
    write(root, f"{RUN}/logs/guest.log", b"booted\n")
    write(root, f"{RUN}/deep/nested/checks.json", b"x" * (bundles.SMALL_LIMIT + 1))
    write(root, f"{RUN}/tools/run.sh", b"#!/bin/sh\n" + b"#" * 9000, executable=True)
    write(root, "docs/rewrite-evidence/p09/summary.txt", b"small\n")
    write(root, "docs/rewrite-evidence/p09/screen.png", b"\x89PNG fake")


class PolicyTests(unittest.TestCase):
    def test_raw_types_and_sizes(self):
        self.assertIsNone(bundles.problem(f"{RUN}/result.json", 100))
        self.assertIn(".gz", bundles.problem(f"{RUN}/record.json.gz", 10))
        self.assertIn(".log", bundles.problem(f"{RUN}/guest.LOG", 10))
        self.assertIn(
            "exceeds", bundles.problem(f"{RUN}/checks.json", bundles.SMALL_LIMIT + 1)
        )
        self.assertIsNone(bundles.problem(f"{RUN}/checks.json", bundles.SMALL_LIMIT))

    def test_summaries_and_hash_lists_have_the_larger_limit(self):
        for name in [
            "README.md",
            "SHA256SUMS",
            "CI-SHA256SUMS",
            "inputs.sha256",
            bundles.MANIFEST,
        ]:
            self.assertIsNone(
                bundles.problem(f"{RUN}/{name}", bundles.INDEX_LIMIT), name
            )
            self.assertIn(
                "exceeds",
                bundles.problem(f"{RUN}/{name}", bundles.INDEX_LIMIT + 1),
                name,
            )

    def test_pinned_files_stay_whatever_their_size(self):
        self.assertIsNone(
            bundles.problem("docs/rewrite-evidence/p03/lifecycle-progress.png", 10**6)
        )
        self.assertIsNone(
            bundles.problem(
                "docs/rewrite-evidence/p04/storage-locks-validation.json", 10**6
            )
        )

    def test_violations_ignore_paths_outside_evidence_and_missing_files(self):
        with tempfile.TemporaryDirectory() as temporary:
            write(temporary, "docs/plans/big.md", b"x" * (bundles.INDEX_LIMIT + 1))
            write(temporary, f"{RUN}/new.log", b"line\n")
            write(temporary, f"{RUN}/ok.json", b"{}\n")
            found = bundles.violations(
                temporary,
                [
                    "docs/plans/big.md",
                    f"{RUN}/new.log",
                    f"{RUN}/ok.json",
                    f"{RUN}/deleted.log",
                ],
            )
        self.assertEqual(found, [f"{RUN}/new.log: raw artifact type .log"])

    def test_units_and_bundle_names(self):
        self.assertEqual(bundles.unit_of(f"{RUN}/deep/a.log"), PurePosixPath(RUN))
        self.assertEqual(
            bundles.unit_of("docs/rewrite-evidence/p09/screen.png"),
            PurePosixPath("docs/rewrite-evidence/p09"),
        )
        self.assertIsNone(bundles.unit_of("docs/rewrite-evidence/README.md"))
        self.assertEqual(
            bundles.bundle_name(RUN), "limeos-evidence-p09-2026-01-01-run.tar.gz"
        )
        self.assertEqual(
            bundles.bundle_name("docs/rewrite-evidence/p09"),
            "limeos-evidence-p09.tar.gz",
        )


class BundleTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name) / "repo"
        self.out = Path(self.temporary.name) / "out"
        fixture(self.root)
        self.before = snapshot(self.root)

    def tearDown(self):
        self.temporary.cleanup()

    def run_cli(self, *argv):
        output, errors = io.StringIO(), io.StringIO()
        with redirect_stdout(output), redirect_stderr(errors):
            code = bundles.main(["--root", str(self.root), *argv])
        return code, output.getvalue() + errors.getvalue()

    def build(self):
        code, output = self.run_cli(
            "build", "--release", "evidence-test", "--out", str(self.out)
        )
        self.assertEqual(code, 0, output)
        return output

    def test_build_moves_only_raw_files_and_records_them(self):
        output = self.build()
        self.assertIn("Built 2 bundles", output)
        remaining = sorted(snapshot(self.root))
        self.assertEqual(
            remaining,
            [
                f"{RUN}/{bundles.MANIFEST}",
                f"{RUN}/README.md",
                f"{RUN}/SHA256SUMS",
                f"{RUN}/result.json",
                f"docs/rewrite-evidence/p09/{bundles.MANIFEST}",
                "docs/rewrite-evidence/p09/summary.txt",
            ],
        )
        self.assertFalse(
            (self.root / RUN / "logs").exists(), "emptied directories are removed"
        )
        manifest = json.loads((self.root / RUN / bundles.MANIFEST).read_text())
        self.assertEqual(manifest["unit"], RUN)
        self.assertEqual(manifest["bundle"]["release"], "evidence-test")
        self.assertEqual(
            [entry["path"] for entry in manifest["files"]],
            [
                "deep/nested/checks.json",
                "logs/guest.log",
                "record.json.gz",
                "tools/run.sh",
            ],
        )
        bundle = self.out / manifest["bundle"]["name"]
        self.assertEqual(manifest["bundle"]["size"], bundle.stat().st_size)
        self.assertEqual(
            manifest["bundle"]["sha256"], bundles.sha256(bundle.read_bytes())
        )
        for entry in manifest["files"]:
            data, _ = self.before[f"{RUN}/{entry['path']}"]
            self.assertEqual(
                (entry["size"], entry["sha256"]), (len(data), bundles.sha256(data))
            )
        loose = json.loads(
            (self.root / "docs/rewrite-evidence/p09" / bundles.MANIFEST).read_text()
        )
        self.assertEqual([entry["path"] for entry in loose["files"]], ["screen.png"])

    def test_bundles_are_byte_for_byte_reproducible(self):
        first = Path(self.temporary.name) / "first"
        second = Path(self.temporary.name) / "second"
        for out in (first, second):
            code, output = self.run_cli(
                "build", "--release", "evidence-test", "--keep-files", "--out", str(out)
            )
            self.assertEqual(code, 0, output)
            for manifest in self.root.rglob(bundles.MANIFEST):
                manifest.unlink()
        self.assertEqual(snapshot(first), snapshot(second))
        with tarfile.open(next(first.glob("*p09-2026*"))) as archive:
            for member in archive:
                self.assertEqual((member.mtime, member.uid, member.gid), (0, 0, 0))
                self.assertEqual((member.uname, member.gname), ("", ""))
                self.assertTrue(member.name.startswith(RUN + "/"))

    def test_restore_gives_back_the_original_bytes_and_modes(self):
        self.build()
        code, output = self.run_cli("verify", "--bundles", str(self.out))
        self.assertEqual(code, 0, output)
        self.assertIn("Verified 5 files from 2 bundles", output)
        code, output = self.run_cli("restore", "--bundles", str(self.out))
        self.assertEqual(code, 0, output)
        restored = snapshot(self.root)
        for manifest in [
            f"{RUN}/{bundles.MANIFEST}",
            f"docs/rewrite-evidence/p09/{bundles.MANIFEST}",
        ]:
            del restored[manifest]
        self.assertEqual(restored, self.before)
        code, output = self.run_cli("restore", "--bundles", str(self.out))
        self.assertEqual(code, 0, "restoring identical files again is a no-op")

    def test_restore_refuses_to_overwrite_changed_files_without_force(self):
        self.build()
        write(self.root, f"{RUN}/logs/guest.log", b"local edit\n")
        code, output = self.run_cli(
            "restore", "--bundles", str(self.out), str(self.root / RUN)
        )
        self.assertEqual(code, 1)
        self.assertIn("exists with different content", output)
        code, output = self.run_cli(
            "restore", "--force", "--bundles", str(self.out), str(self.root / RUN)
        )
        self.assertEqual(code, 0, output)
        self.assertEqual((self.root / RUN / "logs/guest.log").read_bytes(), b"booted\n")

    def test_rebuilding_a_bundled_unit_with_new_raw_files_is_refused(self):
        self.build()
        write(self.root, f"{RUN}/late.log", b"late\n")
        code, output = self.run_cli(
            "build",
            "--release",
            "evidence-test",
            "--out",
            str(self.out),
            str(self.root / RUN),
        )
        self.assertEqual(code, 1)
        self.assertIn("already has", output)

    def rewrite_bundle(self, change):
        self.build()
        manifest = json.loads((self.root / RUN / bundles.MANIFEST).read_text())
        path = self.out / manifest["bundle"]["name"]
        members = []
        with tarfile.open(path) as archive:
            for member in archive:
                members.append((member, archive.extractfile(member).read()))
        members = change(members)
        buffer = io.BytesIO()
        with tarfile.open(
            fileobj=buffer, mode="w", format=tarfile.PAX_FORMAT
        ) as archive:
            for member, data in members:
                member.size = len(data)
                archive.addfile(member, io.BytesIO(data))
        path.write_bytes(gzip.compress(buffer.getvalue(), mtime=0))
        code, output = self.run_cli(
            "verify", "--bundles", str(self.out), str(self.root / RUN)
        )
        self.assertEqual(code, 1)
        return output

    def test_verify_rejects_a_changed_bundle(self):
        output = self.rewrite_bundle(
            lambda members: [(m, d + b"!") for m, d in members]
        )
        self.assertIn("matches neither", output)

    def test_a_recompressed_bundle_with_the_same_tar_stream_still_verifies(self):
        self.build()
        manifest = json.loads((self.root / RUN / bundles.MANIFEST).read_text())
        path = self.out / manifest["bundle"]["name"]
        path.write_bytes(
            gzip.compress(gzip.decompress(path.read_bytes()), compresslevel=1)
        )
        self.assertNotEqual(
            bundles.sha256(path.read_bytes()), manifest["bundle"]["sha256"]
        )
        code, output = self.run_cli(
            "verify", "--bundles", str(self.out), str(self.root / RUN)
        )
        self.assertEqual(code, 0, output)

    def test_verify_rejects_changed_missing_extra_and_unsafe_members(self):
        def accept_any_hash():
            original = bundles.read_bundle

            def patched(manifest, bundle_path):
                manifest = dict(manifest)
                actual = gzip.decompress(Path(bundle_path).read_bytes())
                manifest["bundle"] = {
                    **manifest["bundle"],
                    "tar_sha256": bundles.sha256(actual),
                }
                return original(manifest, bundle_path)

            return patched

        def changed(members):
            return [
                (m, b"tampered" if m.name.endswith("guest.log") else d)
                for m, d in members
            ]

        def missing(members):
            return members[1:]

        def extra(members):
            return members + [(tarfile.TarInfo(f"{RUN}/extra.log"), b"extra")]

        def unsafe(members):
            return members + [(tarfile.TarInfo("../escape"), b"x")]

        expected = {
            changed: "does not match the manifest",
            missing: "missing 1 files",
            extra: "unexpected member",
            unsafe: "unsafe member",
        }
        original = bundles.read_bundle
        bundles.read_bundle = accept_any_hash()
        try:
            for change, message in expected.items():
                with self.subTest(change.__name__):
                    self.assertIn(message, self.rewrite_bundle(change))
                    self.tearDown()
                    self.setUp()
        finally:
            bundles.read_bundle = original


if __name__ == "__main__":
    unittest.main()
