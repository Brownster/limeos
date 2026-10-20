# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Collect terminal attempts without promoting failed or zero-test runs."""
import gzip
import hashlib
import json
from pathlib import Path
import shutil

root = Path(__file__).resolve().parents[2]
state = Path(__file__).resolve().parent
attempts = root / ".cache/main-consolidation/attempts"
out = state / "evidence-progress"
out.mkdir(exist_ok=True)

def copy(source, relative, compressed=False):
    target = out / relative
    target.parent.mkdir(parents=True, exist_ok=True)
    data = source.read_bytes()
    target.write_bytes(gzip.compress(data, mtime=0) if compressed else data)

summary = []
for label in [
    "a1-namespace-baseline", "a1-namespace-native-storage",
    "a1-namespace-native-anchors", "a1-namespace-native-admission",
    "a1-namespace-native-library", "a1-namespace-native-clippy",
    "a1-namespace-candidate", "a1-namespace-candidate-verified",
    "a1-namespace-native-diagnostics", "a1-namespace-diagnostics-clippy",
    "a1-namespace-diagnostic-vm", "a1-namespace-proc-witness",
    "a1-target-uid-probe-tests", "a1-namespace-proc-witness-v2",
    "a1-namespace-proc-witness-768", "a1-namespace-proc-witness-768-diagnostics",
    "a1-namespace-storage-compile", "a1-namespace-storage-runtime",
    "a1-namespace-final-executor", "a1-namespace-final-storage-docs",
    "a1-namespace-final-reference", "a1-namespace-final-reference-fixed",
]:
    directory = attempts / label
    record = json.loads((directory / "record.json").read_text())
    if "exit_code" not in record:
        summary.append({"attempt": label, "terminal": False})
        continue
    for name in ["record.json", "output.log"]:
        copy(directory / name, f"attempts/{label}/{name}.gz", True)
    summary.append({"attempt": label, "terminal": True, "exit_code": record["exit_code"], "guard_triggered": record["guard_triggered"], "inputs_unchanged": record["inputs_unchanged"], "zero_tests_not_qualification": label == "a1-namespace-native-admission"})
    if label.endswith(("baseline", "verified", "diagnostic-vm", "proc-witness", "proc-witness-v2")):
        rows = [json.loads(line.removeprefix("A1_FAILURE ")) for line in (directory / "output.log").read_text().splitlines() if line.startswith("A1_FAILURE ")]
        if rows:
            assert len(rows) == 1
            (out / f"{label}-failure.json").write_text(json.dumps(rows[0], indent=2) + "\n")
for name in ["debian-build", "debian-build-diagnostics"]:
    for filename in ["record.json", "output.log", "container-before.json", "container-after.json", "inputs.sha256", "container-source-check.log"]:
        source = state / name / filename
        if source.is_file():
            copy(source, f"{name}/{filename}.gz", True)
for name in ["candidate", "diagnostic-candidate"]:
    for filename in ["inputs.json", "package-handoff.json"]:
        copy(state / name / filename, f"{name}/{filename}")
    for path in sorted((state / name / "guest").glob("*.py")):
        copy(path, f"{name}/guest/{path.name}")
for filename in ["inputs.json", "proc-witness-inputs.json", "proc-witness-v2-inputs.json", "proc-witness-v3-inputs.json", "lowmem-runner.json", "prepare-candidate-2.log", "prepare-candidate-3-handoff-failure.log", "prepare-candidate-initial-success.log", "prepare-candidate-3.log", "build-debian-initial.py", "build-debian.py", "build-container-diagnostics.py", "prepare-candidate-initial.py", "prepare-candidate.py", "prepare-witness.py", "prepare-witness-initial.py", "prepare-witness-v3.py", "prepare-lowmem.py", "run_768.py", "current_authority_schema.py", "namespace_census.py", "storage-runtime.py", "storage-kernel-runtime.json", "inspect-witness.py"]:
    copy(state / filename, filename)
for path in sorted((state / "proc-witness-guest").glob("*.py")):
    copy(path, f"proc-witness-guest/{path.name}")
for path in sorted((state / "proc-witness-v2-guest").glob("*.py")):
    copy(path, f"proc-witness-v2-guest/{path.name}")
for path in sorted((state / "proc-witness-v3-guest").glob("*.py")):
    copy(path, f"proc-witness-v3-guest/{path.name}")
copy(state / "storage-kernel-runtime.log", "storage-kernel-runtime.log.gz", True)
copy(root / "Cargo.lock", "Cargo.lock.gz", True)
copy(Path(__file__), "collect-progress.py")
(out / "attempts.json").write_text(json.dumps(summary, indent=2) + "\n")
hashes = []
for path in sorted(out.rglob("*")):
    if path.is_file() and path.name != "artifacts.sha256":
        hashes.append(f"{hashlib.sha256(path.read_bytes()).hexdigest()}  {path.relative_to(out)}\n")
(out / "artifacts.sha256").write_text("".join(hashes))
print(json.dumps({"packet": str(out), "terminal_attempts": sum(r["terminal"] for r in summary), "live_attempts": [r["attempt"] for r in summary if not r["terminal"]]}))
