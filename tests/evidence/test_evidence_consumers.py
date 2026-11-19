#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Evidence that code reads must stay in the tree, not in a bundle."""

import fnmatch
import importlib.util
import json
import re
import subprocess
import unittest
from pathlib import Path, PurePosixPath

ROOT = Path(__file__).resolve().parents[2]


def load(relative):
    spec = importlib.util.spec_from_file_location(Path(relative).stem, ROOT / relative)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


bundles = load("scripts/evidence_bundles.py")


def tracked(*paths):
    output = subprocess.run(
        ["git", "ls-files", "-z", "--", *paths],
        cwd=ROOT,
        capture_output=True,
        check=True,
    ).stdout
    return [item.decode() for item in output.split(b"\0") if item]


def bundled():
    """Every repository path that now lives in a bundle."""
    paths = set()
    for manifest in bundles.existing_manifests(ROOT):
        record = json.loads(manifest.read_text())
        unit = PurePosixPath(record["unit"])
        paths.update(str(unit / entry["path"]) for entry in record["files"])
    return paths


def recorder_inputs():
    """(recorder, input path) for every evidence file a VM evidence recorder reads."""
    found = []
    p03 = load("tests/privileged_vm/record_p03_evidence.py")
    found += [
        ("record_p03_evidence", path)
        for path in p03.evidence_inputs(p03.DEFAULT_OUTPUT.parent).values()
    ]
    compose = load("tests/privileged_vm/record_compose_evidence.py")
    found += [
        ("record_compose_evidence", path)
        for path in compose.evidence_inputs(compose.DEFAULT_OUTPUT.parent).values()
    ]
    storage = load("tests/privileged_vm/record_storage_evidence.py")
    for name in storage.SLICES:
        inputs = storage.evidence_inputs(storage.default_output(name).parent, name)
        found += [
            (f"record_storage_evidence --slice {name}", p) for p in inputs.values()
        ]
    dependencies = load("tests/privileged_vm/record_dependencies_evidence.py")
    found += [
        ("record_dependencies_evidence", path)
        for path in dependencies.evidence_inputs(
            dependencies.DOCUMENTED_EVIDENCE
        ).values()
    ]
    return found


class RecorderInputTests(unittest.TestCase):
    def test_every_recorder_declares_its_evidence_inputs(self):
        for script in sorted((ROOT / "tests/privileged_vm").glob("record_*.py")):
            with self.subTest(script.name):
                self.assertIn("def evidence_inputs(", script.read_text())

    def test_recorder_inputs_are_tracked_and_pinned(self):
        files = set(tracked(str(bundles.EVIDENCE)))
        moved = bundled()
        inputs = recorder_inputs()
        self.assertGreater(len(inputs), 20)
        for recorder, path in inputs:
            relative = path.relative_to(ROOT).as_posix()
            with self.subTest(recorder=recorder, path=relative):
                self.assertFalse(relative in moved, "input was moved into a bundle")
                self.assertTrue(relative in files, "input is not tracked")
                self.assertTrue(relative in bundles.PINNED, "input is not pinned")

    def test_documented_dependencies_evidence_directory(self):
        dependencies = load("tests/privileged_vm/record_dependencies_evidence.py")
        guide = (
            dependencies.DOCUMENTED_EVIDENCE / "2026-10-06-acceptance.md"
        ).read_text()
        relative = dependencies.DOCUMENTED_EVIDENCE.relative_to(ROOT).as_posix()
        self.assertIn(f"--evidence {relative}", guide)


EVIDENCE_PATH = re.compile(r"docs/rewrite-evidence/[^\s\"'`<>()\[\],;|]*")


def literal_references():
    """(file, path) for every evidence path written literally in code, tests or CI."""
    found = []
    for name in tracked("tests", "scripts", "packaging", ".github"):
        try:
            text = (ROOT / name).read_text()
        except (UnicodeDecodeError, IsADirectoryError, FileNotFoundError):
            continue
        for match in EVIDENCE_PATH.finditer(text):
            found.append((name, match.group(0).rstrip(".:/*")))
    return found


def matches(template, path):
    """Match a path against an f-string template, one path segment per segment."""
    pattern = re.sub(r"\{[^}]*\}", "*", template).split("/")
    parts = path.split("/")
    return len(pattern) == len(parts) and all(
        fnmatch.fnmatchcase(part, glob) for part, glob in zip(parts, pattern)
    )


class LiteralReferenceTests(unittest.TestCase):
    def test_no_code_names_a_bundled_file_or_emptied_directory(self):
        moved = bundled()
        references = literal_references()
        self.assertTrue(references)
        for name, reference in references:
            with self.subTest(file=name, path=reference):
                if "{" in reference:
                    hits = [path for path in moved if matches(reference, path)]
                    self.assertEqual(hits, [], "template names bundled files")
                    continue
                self.assertFalse(reference in moved, "names a bundled file")
                if not (ROOT / reference).exists():
                    prefix = reference + "/"
                    self.assertFalse(
                        any(path.startswith(prefix) for path in moved),
                        "directory exists only in a bundle",
                    )

    def test_segment_matching(self):
        self.assertTrue(matches("a/{x}-v.json", "a/b-v.json"))
        self.assertFalse(matches("a/{x}-v.json", "a/b/c-v.json"))


if __name__ == "__main__":
    unittest.main()
