#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Evidence that code reads must stay in the tree, not in a bundle."""

import ast
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


RECORDERS = sorted((ROOT / "tests/privileged_vm").glob("record_*.py"))


def recorder_files():
    """(recorder, kind, path) for every evidence file a VM evidence recorder reads
    or writes, in its default and documented invocations."""
    found = []

    def add(label, inputs, outputs):
        found.extend((label, "input", path) for path in inputs.values())
        found.extend((label, "output", path) for path in outputs.values())

    p03 = load("tests/privileged_vm/record_p03_evidence.py")
    add(
        "record_p03_evidence",
        p03.evidence_inputs(p03.DEFAULT_OUTPUT.parent),
        p03.evidence_outputs(p03.DEFAULT_OUTPUT),
    )
    compose = load("tests/privileged_vm/record_compose_evidence.py")
    add(
        "record_compose_evidence",
        compose.evidence_inputs(
            compose.DEFAULT_OUTPUT.parent, checks=compose.DEFAULT_CHECKS
        ),
        compose.evidence_outputs(compose.DEFAULT_OUTPUT, reference=False),
    )
    reference = compose.REFERENCE_INVOCATION
    add(
        "record_compose_evidence --reference-profile",
        compose.evidence_inputs(
            reference["output"].parent, reference["vm_result"], reference["checks"]
        ),
        compose.evidence_outputs(reference["output"], reference=True),
    )
    storage = load("tests/privileged_vm/record_storage_evidence.py")
    for name in storage.SLICES:
        output = storage.default_output(name)
        add(
            f"record_storage_evidence --slice {name}",
            storage.evidence_inputs(output.parent, name),
            storage.evidence_outputs(output, name),
        )
    dependencies = load("tests/privileged_vm/record_dependencies_evidence.py")
    add(
        "record_dependencies_evidence",
        dependencies.evidence_inputs(dependencies.DOCUMENTED_EVIDENCE),
        dependencies.evidence_outputs(dependencies.DOCUMENTED_EVIDENCE),
    )
    return found


DECLARATIONS = {"evidence_inputs", "evidence_outputs", "default_output"}
PATH_METHODS = {"with_name", "with_suffix", "joinpath", "resolve", "absolute"}
SOURCES = {
    "DEFAULT_OUTPUT",
    "DEFAULT_CHECKS",
    "DOCUMENTED_EVIDENCE",
    "REFERENCE_INVOCATION",
}
# Command-line values that are not evidence paths. Every other args.<name> is
# treated as an evidence path and must reach the recorder through evidence_inputs().
EXTERNAL_OPTIONS = {
    "args.bundle",
    "args.artifacts",
    "args.source_commit",
    "args.slice",
    "args.reference_profile",
    "args.container_fixture_commit",
    "args.storage_fixture_commit",
}


def dotted(node):
    if isinstance(node, ast.Name):
        return node.id
    if isinstance(node, ast.Attribute):
        base = dotted(node.value)
        return base and f"{base}.{node.attr}"
    return None


def bound(target):
    """Names (or dotted attributes) an assignment target binds."""
    if isinstance(target, (ast.Tuple, ast.List)):
        return [name for element in target.elts for name in bound(element)]
    if isinstance(target, ast.Starred):
        return bound(target.value)
    name = dotted(target)
    return [name] if name else []


def undeclared_access(source):
    """Calls in a recorder that touch an evidence path not from its declarations.

    A path is "evidence" when it is built from the output or evidence
    arguments, the documented defaults, or a literal under docs/rewrite-evidence.
    Every read or write of such a path must use a value returned by
    evidence_inputs() or evidence_outputs().
    """
    tree = ast.parse(source)
    problems = []
    for function in ast.walk(tree):
        if not isinstance(function, ast.FunctionDef) or function.name in DECLARATIONS:
            continue
        tainted, declared = set(SOURCES), set()

        def is_declared(node, declared=declared):
            if isinstance(node, ast.Name):
                return node.id in declared
            return isinstance(node, ast.Subscript) and is_declared(node.value)

        def taints(node, tainted=tainted):
            if is_declared(node):
                return False
            if isinstance(node, ast.Attribute) and node.attr in ("name", "stem"):
                return False
            if isinstance(node, ast.Call):
                # A call's result is data, not a path, unless it derives a path.
                method = node.func.attr if isinstance(node.func, ast.Attribute) else ""
                return method in PATH_METHODS and taints(node.func.value)
            name = dotted(node) or ""
            if name in tainted:
                return True
            option = ".".join(name.split(".")[:2])
            if name.startswith("args.") and option not in EXTERNAL_OPTIONS:
                return True
            if isinstance(node, ast.Constant) and "rewrite-evidence" in str(node.value):
                return True
            return any(taints(child) for child in ast.iter_child_nodes(node))

        assignments = []
        for node in ast.walk(function):
            if isinstance(node, ast.Assign):
                assignments.append((node.targets, node.value))
            elif isinstance(node, (ast.AnnAssign, ast.AugAssign, ast.NamedExpr)):
                if node.value is not None:
                    assignments.append(([node.target], node.value))
            elif isinstance(node, (ast.For, ast.comprehension)):
                assignments.append(([node.target], node.iter))
        changed = True
        while changed:
            changed = False
            for targets, value in assignments:
                names = [name for target in targets for name in bound(target)]
                if (
                    isinstance(value, ast.Call) and dotted(value.func) in DECLARATIONS
                ) or is_declared(value):
                    new = set(names) - declared
                    declared |= new
                elif taints(value):
                    new = set(names) - tainted
                    tainted |= new
                else:
                    new = set()
                changed = changed or bool(new)
        for call in ast.walk(function):
            if not isinstance(call, ast.Call):
                continue
            callee = dotted(call.func) or ""
            if callee.split(".")[-1] in DECLARATIONS | {"add_argument"}:
                continue
            receiver = call.func.value if isinstance(call.func, ast.Attribute) else None
            arguments = [*call.args, *(k.value for k in call.keywords)]
            if (receiver is not None and taints(receiver)) or any(
                taints(argument) for argument in arguments
            ):
                problems.append(
                    f"{function.name}:{call.lineno}: {ast.unparse(call)[:80]}"
                )
    return problems


class RecorderInputTests(unittest.TestCase):
    def test_every_recorder_declares_its_evidence_files(self):
        for script in RECORDERS:
            with self.subTest(script.name):
                text = script.read_text()
                self.assertIn("def evidence_inputs(", text)
                self.assertIn("def evidence_outputs(", text)

    def test_recorders_touch_evidence_only_through_their_declarations(self):
        self.assertEqual(len(RECORDERS), 4)
        for script in RECORDERS:
            with self.subTest(script.name):
                self.assertEqual(undeclared_access(script.read_text()), [])

    def test_an_undeclared_read_is_caught(self):
        """The two mutations the second review used, applied in memory."""
        p03 = (ROOT / "tests/privileged_vm/record_p03_evidence.py").read_text()
        empty = re.sub(
            r"(def evidence_inputs\(evidence\):\n    \"\"\".*?\"\"\"\n)    return \{.*?\n    \}\n",
            r"\1    return {}\n",
            p03,
            count=1,
            flags=re.DOTALL,
        )
        for key, name in [
            ("vm", "lifecycle-vm-result.json"),
            ("browser", "lifecycle-browser-result.json"),
            ("log", "rust-lifecycle-tests.txt"),
        ]:
            empty = empty.replace(f'inputs["{key}"]', f'(evidence / "{name}")')
        self.assertIn("return {}", empty)
        self.assertEqual(len(undeclared_access(empty)), 3)
        compose = (ROOT / "tests/privileged_vm/record_compose_evidence.py").read_text()
        anchor = "    directory = args.output.parent\n"
        extra = compose.replace(
            anchor,
            anchor
            + '    extra = (directory / "reference-vm-result.json").read_text()\n',
        )
        self.assertNotEqual(extra, compose)
        found = undeclared_access(extra)
        self.assertEqual(len(found), 1)
        self.assertIn("reference-vm-result.json", found[0])
        sneaky = compose.replace(
            anchor, anchor + "    extra = json.load(open(directory / 'x.json'))\n"
        )
        self.assertTrue(undeclared_access(sneaky))

    def test_every_assignment_form_carries_the_evidence_path(self):
        compose = (ROOT / "tests/privileged_vm/record_compose_evidence.py").read_text()
        anchor = "    directory = args.output.parent\n"
        forms = {
            "annotated": '    extra: Path = directory / "x.json"\n    extra.read_text()\n',
            "walrus": '    (extra := directory / "x.json")\n    extra.read_text()\n',
            "augmented": '    extra = ROOT\n    extra /= "docs/rewrite-evidence/x.json"\n'
            "    extra.read_text()\n",
            "loop": '    for extra in [directory / "x.json"]:\n        extra.read_text()\n',
            "comprehension": '    [p.read_text() for p in [directory / "x.json"]]\n',
            "tuple": '    extra, other = directory / "x.json", 1\n    extra.read_text()\n',
            "undeclared option": "    args.checks.read_text()\n",
        }
        for label, code in forms.items():
            with self.subTest(label):
                mutated = compose.replace(anchor, anchor + code)
                self.assertNotEqual(mutated, compose)
                self.assertTrue(undeclared_access(mutated), label)

    def test_recorder_files_are_tracked_and_pinned(self):
        files = set(tracked(str(bundles.EVIDENCE)))
        moved = bundled()
        found = recorder_files()
        self.assertGreater(len(found), 40)
        for recorder, kind, path in found:
            relative = path.relative_to(ROOT).as_posix()
            with self.subTest(recorder=recorder, kind=kind, path=relative):
                self.assertFalse(relative in moved, f"{kind} was moved into a bundle")
                self.assertTrue(relative in files, f"{kind} is not tracked")
                self.assertTrue(relative in bundles.PINNED, f"{kind} is not pinned")

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
