# /// script
# dependencies = []
# ///
"""Collect local notification evidence; never import or execute reference code."""

import ast
import gzip
import hashlib
import json
import re
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
REFERENCE = Path("/home/marc/Documents/github/pi-health")
DEST = ROOT / "docs/rewrite-evidence/p04/2026-10-18-stack-notifications"
SOURCE = "b72c14c"


def command(*args: str, cwd: Path = ROOT) -> str:
    return subprocess.check_output(args, cwd=cwd, text=True).strip()


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def write_hashes(name: str, root: Path, paths: list[str]) -> None:
    (DEST / name).write_text(
        "".join(f"{digest(root / path)}  {path}\n" for path in sorted(paths))
    )


DEST.mkdir(parents=True, exist_ok=True)
source = command("git", "rev-parse", SOURCE)
assert command("git", "rev-parse", "HEAD") == source
assert command("git", "rev-parse", "HEAD", cwd=REFERENCE) == (
    "80593b29443ea9b81eb60ed4e5c3c3ac7a236204"
)
product = command("git", "diff", "--name-only", "7df3f75", SOURCE).splitlines()
assert len(product) == 8  # seven product paths plus the earlier design commit
code = [path for path in product if not path.startswith("docs/")]
assert len(code) == 7
assert not command("git", "diff", "HEAD", "--", *code)

# Statically compare the hand-transcribed event table with literal reference data.
# ast.literal_eval sees only literal expressions; no reference module is imported.
tree = ast.parse((REFERENCE / "stack_notifications_service.py").read_text())
assignments = {
    target.id: node.value
    for node in tree.body
    if isinstance(node, ast.Assign)
    for target in node.targets
    if isinstance(target, ast.Name)
}
event_map = ast.literal_eval(assignments["_EVENT_MAP"])
quiet_call = assignments["QUIET_EVENTS"]
assert isinstance(quiet_call, ast.Call)
quiet = ast.literal_eval(quiet_call.args[0])
fixture = json.loads(
    (ROOT / "tests/fixtures/deployment/stack-notifications.json").read_text()
)
assert len(fixture["aliases"]) == len(event_map) == 16
assert {row[0].lower() for row in fixture["aliases"]} == set(event_map)
for raw, normalized, forwarded in fixture["aliases"]:
    expected = event_map[raw.lower()]
    assert normalized == (None if expected == "test" else expected)
    assert forwarded == (expected in quiet)
(DEST / "reference-map.json").write_text(
    json.dumps(
        {
            "method": "AST literal extraction only; no frozen module/test execution",
            "event_map": event_map,
            "quiet": sorted(quiet),
            "matched_fixture_aliases": 16,
            "native_test_credit": 0,
        },
        indent=2,
    )
    + "\n"
)

tracked = command("git", "ls-files").splitlines()
inputs = [
    path
    for path in tracked
    if path in {"Cargo.toml", "Cargo.lock", "rust-toolchain.toml"}
    or path.startswith(("crates/", "bins/", "tests/fixtures/", ".cargo/"))
]
write_hashes("SOURCE_INPUTS.sha256", ROOT, inputs)
reference_paths = [
    "stack_notifications_service.py",
    "tests/test_stack_notifications.py",
    "tests/test_stack_notifications_api.py",
    "alert_daemon.py",
    "alert_notifier.py",
    "mattermost_integration_service.py",
    "integrations_manager.py",
    "limeos_packages.py",
    "update_service.py",
    "update_scheduler.py",
    "frontend/src/components/integrations/stack-notifications-card.tsx",
    "frontend/src/lib/integrations.ts",
]
write_hashes("REFERENCE_INPUTS.sha256", REFERENCE, reference_paths)

logs = sorted((ROOT / ".cache/stack-notifications").glob("*.log"))
test_results = {}
for path in logs:
    (DEST / (path.name + ".gz")).write_bytes(gzip.compress(path.read_bytes(), mtime=0))
    test_results[path.name] = re.findall(
        r"test result: (.*)", path.read_text(errors="replace")
    )
combined = (ROOT / ".cache/stack-notifications/test-combined.log").read_text()
counts = [int(n) for n in re.findall(r"test result: ok\. (\d+) passed", combined)]
assert counts == [135, 19, 294, 6, 0, 0, 0, 0]
assert sum(counts) == 454

harness = ROOT / ".cache/stream-b-leaf"
for path in sorted(harness.rglob("*")):
    if path.is_file() and path.name in {"Cargo.toml", "Cargo.lock", "lib.rs"}:
        target = DEST / "harness" / path.relative_to(harness)
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(path.read_bytes())
assert len(list((DEST / "harness").rglob("*.*"))) == 10
(DEST / "collector.py").write_bytes(Path(__file__).read_bytes())
(DEST / "manifest.json").write_text(
    json.dumps(
        {
            "source": source,
            "base": command("git", "rev-parse", "7df3f75"),
            "branch": "engineer/p04-stack-notifications",
            "reference": command("git", "rev-parse", "HEAD", cwd=REFERENCE),
            "rustc": command("rustc", "-vV"),
            "cargo": command("cargo", "--version"),
            "uv": command("uv", "--version"),
            "product_paths": code,
            "source_input_count": len(inputs),
            "reference_input_count": len(reference_paths),
            "harness_file_count": 10,
            "combined_native_tests": 454,
            "new_native_tests": 30,
            "fixture_aliases": 16,
            "fixture_payloads": 8,
            "test_results": test_results,
            "environment": {
                "CARGO_BUILD_JOBS": "1",
                "CARGO_INCREMENTAL": "0",
                "RUST_TEST_THREADS": "1",
                "CARGO_TARGET_DIR": "/home/marc/Documents/github/lime-os/.cache/rewrite-pool-dependencies/target",
                "TMPDIR": "/home/marc/Documents/github/lime-os/.cache/tmp",
                "CCACHE_DIR": "/home/marc/Documents/github/lime-os/.cache/ccache",
                "UV_CACHE_DIR": "/home/marc/Documents/github/lime-os/.cache/uv",
            },
            "qualification": "local leaf checks only; no installed/API/UI/hardware or delivery proof",
        },
        indent=2,
    )
    + "\n"
)
write_hashes(
    "SHA256SUMS",
    DEST,
    [
        str(path.relative_to(DEST))
        for path in DEST.rglob("*")
        if path.is_file() and path.name != "SHA256SUMS"
    ],
)
print(f"Collected source {source}, {len(inputs)} source inputs, 454 native cases")
