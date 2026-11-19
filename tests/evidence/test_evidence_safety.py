#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Evidence bundle safety: guard, pins, names, ordering, restore and errors."""

import gzip
import io
import json
import os
import shutil
import subprocess
import sys
import tarfile
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))
from test_evidence_bundles import RUN, bundles, fixture, snapshot, write

ROOT = Path(__file__).resolve().parents[2]
AREA = "docs/rewrite-evidence/p09"


def git(root, *argv):
    subprocess.run(["git", *argv], cwd=root, check=True, capture_output=True)


class Tree(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        base = Path(self.temporary.name)
        self.root = base / "repo"
        self.out = base / "out"
        self.outside = base / "outside"
        self.outside.mkdir()
        fixture(self.root)

    def tearDown(self):
        self.temporary.cleanup()

    def cli(self, *argv):
        output, errors = io.StringIO(), io.StringIO()
        with redirect_stdout(output), redirect_stderr(errors):
            code = bundles.main(["--root", str(self.root), *argv])
        return code, output.getvalue(), errors.getvalue()

    def build(self, *units):
        code, _out, err = self.cli(
            "build", "--release", "t", "--out", str(self.out), *units
        )
        self.assertEqual(code, 0, err)

    def restore(self, *argv):
        return self.cli("restore", "--bundles", str(self.out), *argv)

    def manifest(self, unit=RUN):
        return self.root / unit / bundles.MANIFEST

    def rewrite_manifest(self, change, unit=RUN):
        data = json.loads(self.manifest(unit).read_text())
        change(data)
        self.manifest(unit).write_text(json.dumps(data))

    def replace_bundle(self, members, unit=RUN):
        """Write a crafted tar as the unit's bundle and record its hashes."""
        buffer = io.BytesIO()
        with tarfile.open(fileobj=buffer, mode="w", format=tarfile.PAX_FORMAT) as tar:
            for info, data in members:
                tar.addfile(info, io.BytesIO(data) if data is not None else None)
        raw = buffer.getvalue()
        compressed = gzip.compress(raw, mtime=0)
        name = bundles.bundle_name(unit)
        (self.out / name).write_bytes(compressed)

        def change(manifest):
            manifest["bundle"]["sha256"] = bundles.sha256(compressed)
            manifest["bundle"]["tar_sha256"] = bundles.sha256(raw)

        self.rewrite_manifest(change, unit)

    def original_members(self, unit=RUN):
        with tarfile.open(self.out / bundles.bundle_name(unit)) as tar:
            return [(m, tar.extractfile(m).read()) for m in tar]


class PinTests(unittest.TestCase):
    def test_pins_are_exact_paths(self):
        for path in bundles.PINNED:
            self.assertFalse(set("*?[") & set(path), path)

    def test_a_pin_does_not_cover_deeper_or_similar_paths(self):
        harness = "docs/rewrite-evidence/arm64/2026-11-05-pi-night-prep/harness"
        self.assertIsNone(bundles.problem(f"{harness}/build_arm64_binaries.py", 10**6))
        self.assertIsNotNone(bundles.problem(f"{harness}/out/run.log", 50 << 20))
        self.assertIsNotNone(bundles.problem(f"{harness}/other.json", 50 << 20))
        self.assertIsNotNone(
            bundles.problem(
                "docs/rewrite-evidence/p04/storage-x/y-validation.json", 10**6
            )
        )
        self.assertIsNotNone(
            bundles.problem(
                "docs/rewrite-evidence/p04/storage-new-validation.json", 10**6
            )
        )


class GuardTests(unittest.TestCase):
    def test_quoted_names_are_still_checked(self):
        with tempfile.TemporaryDirectory() as temporary:
            names = [f"{RUN}/tab\tname.log", f"{RUN}/naïve.png", f"{RUN}/ok.json"]
            for name in names:
                write(temporary, name, b"x")
            git(temporary, "init", "--quiet")
            git(temporary, "add", ".")
            found = bundles.tracked_violations(temporary)
        self.assertEqual(
            sorted(found),
            [
                f"{RUN}/naïve.png: raw artifact type .png",
                f"{RUN}/tab\tname.log: raw artifact type .log",
            ],
        )

    def test_outside_a_repository_there_is_nothing_to_check(self):
        with tempfile.TemporaryDirectory() as temporary:
            self.assertIsNone(bundles.tracked_violations(temporary))

    def test_check_repository_runs_the_guard(self):
        with tempfile.TemporaryDirectory() as temporary:
            tree = Path(temporary)
            for name in ["scripts/check_repository.py", "scripts/evidence_bundles.py"]:
                (tree / name).parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(ROOT / name, tree / name)
            for manifest in ROOT.glob("crates/*/Cargo.toml"):
                target = tree / manifest.relative_to(ROOT)
                target.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(manifest, target)
            shutil.copyfile(ROOT / "Cargo.lock", tree / "Cargo.lock")
            write(tree, f"{RUN}/README.md", b"# Run\n")
            git(tree, "init", "--quiet")
            git(tree, "add", ".")

            def check():
                return subprocess.run(
                    [sys.executable, "scripts/check_repository.py"],
                    cwd=tree,
                    env={**os.environ, "PYTHONDONTWRITEBYTECODE": "1"},
                    capture_output=True,
                    text=True,
                    check=False,
                )

            self.assertEqual(check().returncode, 0, check().stderr)
            write(tree, f"{RUN}/naïve shot.png", b"png")
            git(tree, "add", ".")
            result = check()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("evidence policy violation", result.stderr)
        self.assertIn("naïve shot.png: raw artifact type .png", result.stderr)


class NameTests(Tree):
    def test_names_are_one_to_one_and_dashed_areas_are_refused(self):
        units = [f"{AREA}", f"{AREA}/a-b", f"{AREA}/a", "docs/rewrite-evidence/p10/b"]
        names = [bundles.bundle_name(unit) for unit in units]
        self.assertEqual(len(set(names)), len(names))
        with self.assertRaises(bundles.EvidenceError):
            bundles.bundle_name("docs/rewrite-evidence/p09-a/b")

    def test_a_colliding_unit_stops_the_build_before_anything_is_written(self):
        write(self.root, "docs/rewrite-evidence/p09-a/b/x.log", b"one\n")
        write(self.root, f"{AREA}/a-b/x.log", b"two\n")
        before = snapshot(self.root)
        code, _out, err = self.cli("build", "--release", "t", "--out", str(self.out))
        self.assertEqual(code, 1)
        self.assertIn("ambiguous", err)
        self.assertEqual(snapshot(self.root), before)
        self.assertFalse(self.out.exists())


class BuildOrderTests(Tree):
    def test_no_input_is_deleted_unless_every_bundle_verifies(self):
        before = snapshot(self.root)
        original = bundles.read_bundle
        calls = []

        def failing(manifest, path):
            calls.append(manifest["unit"])
            if manifest["unit"] == RUN:
                raise bundles.EvidenceError("simulated verification failure")
            return original(manifest, path)

        with patch.object(bundles, "read_bundle", failing):
            code, _out, _err = self.cli(
                "build", "--release", "t", "--out", str(self.out)
            )
        self.assertEqual(code, 1)
        self.assertIn(AREA, calls, "the first unit was bundled and verified")
        self.assertEqual(snapshot(self.root), before, "nothing deleted, no manifest")
        self.assertEqual(
            sorted(p.name for p in self.out.iterdir()), [bundles.bundle_name(AREA)]
        )

    def test_each_bundle_is_reread_from_disk_before_inputs_go(self):
        seen = []
        original = bundles.read_bundle

        def recording(manifest, path):
            seen.append(
                (
                    manifest["unit"],
                    Path(path).name,
                    (self.root / RUN / "logs/guest.log").exists(),
                )
            )
            return original(manifest, path)

        with patch.object(bundles, "read_bundle", recording):
            self.build()
        for unit, name, input_present in seen:
            self.assertTrue(
                input_present, f"{unit} input deleted before {name} verified"
            )
        self.assertIn((RUN, bundles.bundle_name(RUN), True), seen)

    def test_loose_files_in_a_bundled_area_get_specific_advice(self):
        self.build()
        write(self.root, f"{AREA}/later.log", b"later\n")
        code, _out, err = self.cli("build", "--release", "t", "--out", str(self.out))
        self.assertEqual(code, 1)
        self.assertIn("loose raw file", err)
        self.assertIn(f"{AREA}/YYYY-MM-DD-<name>/", err)

    def test_modes_come_from_the_git_index(self):
        script = self.root / RUN / "tools/run.sh"
        git(self.root, "init", "--quiet")
        git(self.root, "add", ".")
        git(self.root, "update-index", "--chmod=+x", f"{RUN}/tools/run.sh")
        git(self.root, "update-index", "--chmod=-x", f"{RUN}/logs/guest.log")
        script.chmod(0o644)  # as seen on a noexec mount or a lossy copy
        (self.root / RUN / "logs/guest.log").chmod(0o755)
        self.build(str(self.root / RUN))
        modes = {m.name: m.mode for m, _ in self.original_members()}
        self.assertEqual(modes[f"{RUN}/tools/run.sh"], 0o755)
        self.assertEqual(modes[f"{RUN}/logs/guest.log"], 0o644)

    def test_without_git_the_owner_execute_bit_decides(self):
        (self.root / RUN / "logs/guest.log").chmod(0o744)
        (self.root / RUN / "tools/run.sh").chmod(0o655)
        self.build(str(self.root / RUN))
        modes = {m.name: m.mode for m, _ in self.original_members()}
        self.assertEqual(modes[f"{RUN}/logs/guest.log"], 0o755)
        self.assertEqual(modes[f"{RUN}/tools/run.sh"], 0o644)


class RestoreSafetyTests(Tree):
    def setUp(self):
        super().setUp()
        self.build()
        self.built = snapshot(self.root)

    def test_a_symlinked_parent_directory_is_refused(self):
        (self.root / RUN / "logs").symlink_to(self.outside, target_is_directory=True)
        code, _out, err = self.restore("--force")
        self.assertEqual(code, 1)
        self.assertIn("symbolic link", err)
        self.assertEqual(list(self.outside.iterdir()), [])
        self.assertFalse(
            (self.root / RUN / "record.json.gz").exists(), "nothing written"
        )

    def test_a_dangling_symlink_destination_is_refused(self):
        target = self.outside / "planted"
        (self.root / RUN / "record.json.gz").symlink_to(target)
        code, _out, err = self.restore("--force")
        self.assertEqual(code, 1)
        self.assertIn("symbolic link", err)
        self.assertFalse(target.exists())

    def test_identical_and_hardlinked_files_are_left_untouched(self):
        self.assertEqual(self.restore()[0], 0)
        guest = self.root / RUN / "logs/guest.log"
        link = self.outside / "guest-link"
        os.link(guest, link)
        before = os.stat(guest)
        os.chmod(guest, 0o640)  # same content and execute bit
        code, out, err = self.restore()
        self.assertEqual(code, 0, err)
        self.assertIn("(5 already in place)", out)
        after = os.stat(guest)
        self.assertEqual(
            (after.st_ino, after.st_mtime_ns), (before.st_ino, before.st_mtime_ns)
        )
        self.assertEqual(after.st_mode & 0o777, 0o640)
        self.assertEqual(os.stat(link).st_ino, after.st_ino)

    def test_force_replaces_a_hardlinked_file_without_writing_through(self):
        self.assertEqual(self.restore()[0], 0)
        guest = self.root / RUN / "logs/guest.log"
        guest.unlink()
        link = self.outside / "shared"
        link.write_bytes(b"someone else's file\n")
        os.link(link, guest)
        self.assertEqual(self.restore()[0], 1, "differs without --force")
        code, _out, err = self.restore("--force")
        self.assertEqual(code, 0, err)
        self.assertEqual(guest.read_bytes(), b"booted\n")
        self.assertEqual(link.read_bytes(), b"someone else's file\n")

    def test_a_unit_outside_the_evidence_root_is_refused(self):
        self.rewrite_manifest(lambda m: m.update(unit="docs/other/run"))
        code, _out, err = self.restore()
        self.assertEqual(code, 1)
        self.assertIn("not under docs/rewrite-evidence", err)

    def test_a_unit_with_dot_dot_is_refused(self):
        self.rewrite_manifest(lambda m: m.update(unit=f"{AREA}/../p09/x"))
        self.assertEqual(self.restore()[0], 1)

    def test_a_manifest_must_live_in_its_unit(self):
        other = f"{AREA}/2026-01-02-other"

        def move(manifest):
            manifest["unit"] = other
            manifest["bundle"]["name"] = bundles.bundle_name(other)

        self.rewrite_manifest(move)
        before = snapshot(self.root)
        code, _out, err = self.restore()
        self.assertEqual(code, 1)
        self.assertIn("is not in that directory", err)
        self.assertEqual(snapshot(self.root), before)

    def test_file_paths_in_a_manifest_must_stay_inside_the_unit(self):
        for bad in ["../escape.log", "/etc/passwd", "a/./b.log", ""]:
            with self.subTest(bad):
                self.rewrite_manifest(lambda m, bad=bad: m["files"][0].update(path=bad))
                code, _out, err = self.restore()
                self.assertEqual(code, 1)
                self.assertIn("bad file path", err)

    def test_unsafe_tar_members_are_refused(self):
        members = self.original_members()
        cases = {}
        link = tarfile.TarInfo(f"{RUN}/logs/guest.log")
        link.type, link.linkname = tarfile.SYMTYPE, "/etc/passwd"
        cases["symlink"] = link
        hard = tarfile.TarInfo(f"{RUN}/logs/guest.log")
        hard.type, hard.linkname = tarfile.LNKTYPE, f"{RUN}/record.json.gz"
        cases["hardlink"] = hard
        device = tarfile.TarInfo(f"{RUN}/logs/guest.log")
        device.type, device.devmajor, device.devminor = tarfile.CHRTYPE, 1, 3
        cases["device"] = device
        absolute = tarfile.TarInfo("/tmp/evidence-escape.log")
        absolute.size = 1
        cases["absolute"] = absolute
        for label, info in cases.items():
            with self.subTest(label):
                data = b"x" if label == "absolute" else None
                kept = [(m, d) for m, d in members if m.name != f"{RUN}/logs/guest.log"]
                self.replace_bundle(kept + [(info, data)])
                code, _out, err = self.restore("--force")
                self.assertEqual(code, 1)
                self.assertIn("unsafe member", err)
                self.assertFalse((self.root / RUN / "record.json.gz").exists())
                self.assertFalse(Path("/tmp/evidence-escape.log").exists())


class PartialRestoreTests(Tree):
    def setUp(self):
        super().setUp()
        self.build()
        self.built = snapshot(self.root)

    def test_a_missing_bundle_stops_restore_before_any_write(self):
        (self.out / bundles.bundle_name(RUN)).unlink()
        code, _out, _err = self.restore()
        self.assertEqual(code, 1)
        self.assertEqual(snapshot(self.root), self.built)

    def test_a_conflict_in_one_unit_stops_every_unit(self):
        write(self.root, f"{RUN}/logs/guest.log", b"local edit\n")
        expected = snapshot(self.root)
        code, _out, _err = self.restore()
        self.assertEqual(code, 1)
        self.assertEqual(snapshot(self.root), expected)

    def test_a_mid_write_failure_removes_what_this_run_created(self):
        original = bundles.write_file
        calls = []

        def failing(*args):
            calls.append(args[1])
            if len(calls) == 3:
                raise OSError("simulated disk full")
            return original(*args)

        with patch.object(bundles, "write_file", failing):
            code, _out, err = self.restore()
        self.assertEqual(code, 1)
        self.assertIn("simulated disk full", err)
        self.assertEqual(snapshot(self.root), self.built)
        for directory in ["logs", "deep", "tools"]:
            self.assertFalse((self.root / RUN / directory).exists(), directory)


class ErrorTests(Tree):
    def setUp(self):
        super().setUp()
        self.build()
        self.built = snapshot(self.root)

    def assert_one_line_error(self, before, argv, text):
        code, _out, err = self.cli(*argv)
        self.assertEqual(code, 1)
        self.assertEqual(len(err.strip().splitlines()), 1, err)
        self.assertIn(text, err)
        self.assertEqual(snapshot(self.root), before, "nothing written")

    def test_a_truncated_bundle(self):
        path = self.out / bundles.bundle_name(RUN)
        path.write_bytes(path.read_bytes()[:40])
        for command in ("verify", "restore"):
            with self.subTest(command):
                self.assert_one_line_error(
                    self.built,
                    (command, "--bundles", str(self.out)),
                    "unreadable bundle",
                )

    def test_a_manifest_without_files(self):
        self.rewrite_manifest(lambda m: m.pop("files"))
        before = snapshot(self.root)
        for command in ("verify", "restore"):
            with self.subTest(command):
                self.assert_one_line_error(
                    before, (command, "--bundles", str(self.out)), "unreadable manifest"
                )

    def test_a_manifest_that_is_not_json(self):
        self.manifest().write_text("{")
        self.assert_one_line_error(
            snapshot(self.root),
            ("restore", "--bundles", str(self.out)),
            "unreadable manifest",
        )


if __name__ == "__main__":
    unittest.main()
