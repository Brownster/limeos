# /// script
# dependencies = []
# ///
"""Collect source-bound local evidence for MergerFS document proposals."""

import gzip
import hashlib
import json
import re
import subprocess
import sys
from pathlib import Path

ROOT = next(
    parent
    for parent in Path(__file__).resolve().parents
    if (parent / "Cargo.toml").is_file() and (parent / ".git").exists()
)
PACKET = ROOT / "docs/rewrite-evidence/p04/2026-10-18-pool-document-edits"
SOURCE = "e43ac89560faadfb149d036ce5a6e636acb84cb5"
BASE = "6eb3b37"
REFERENCE = Path("/home/marc/Documents/github/pi-health")


def command(*args: str, root: Path = ROOT) -> str:
    return subprocess.check_output(args, cwd=root, text=True).strip()


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def hashes(name: str, root: Path, paths: list[str]) -> None:
    (PACKET / name).write_text(
        "".join(f"{digest(root / path)}  {path}\n" for path in sorted(paths))
    )


PACKET.mkdir(parents=True, exist_ok=True)
product = [
    "crates/domain/src/pools.rs",
    "crates/domain/src/pools/document.rs",
    "crates/domain/src/pools/document/tests.rs",
]
assert not command("git", "diff", SOURCE, "--", *product)
assert sorted(
    command(
        "git",
        "diff",
        "--name-only",
        BASE,
        SOURCE,
        "--",
        "crates",
        "bins",
        "frontend",
        "tests",
        "Cargo.toml",
        "Cargo.lock",
    ).splitlines()
) == sorted(product)
tracked = command("git", "ls-files").splitlines()
inputs = [
    path
    for path in tracked
    if path in {"Cargo.toml", "Cargo.lock", "rust-toolchain.toml"}
    or path.startswith(("crates/domain/", "tests/fixtures/", ".cargo/"))
]
hashes("inputs.sha256", ROOT, inputs)
assert (
    command("git", "rev-parse", "HEAD", root=REFERENCE)
    == "80593b29443ea9b81eb60ed4e5c3c3ac7a236204"
)
reference = ["storage_plugins/mergerfs_plugin.py", "tests/test_mergerfs_plugin.py"]
assert not command("git", "diff", "HEAD", "--", *reference, root=REFERENCE)
hashes("reference-inputs.sha256", REFERENCE, reference)

results = {}
for path in sorted((ROOT / ".cache/pool-document").glob("*.log")):
    (PACKET / (path.name + ".gz")).write_bytes(
        gzip.compress(path.read_bytes(), mtime=0)
    )
    results[path.name] = re.findall(
        r"test result: (.*)", path.read_text(errors="replace")
    )
verified = (ROOT / ".cache/pool-document/test-verified.log").read_text()
assert re.findall(r"test result: ok\. (\d+) passed", verified) == ["207", "7"]
red = (ROOT / ".cache/pool-document/quoted-target-red.log").read_text()
assert "FAILED. 0 passed; 1 failed" in red
(PACKET / "red-regression.patch").write_text(
    subprocess.check_output(
        [
            "git",
            "diff",
            "340d7e1",
            SOURCE,
            "--",
            "crates/domain/src/pools/document/tests.rs",
        ],
        cwd=ROOT,
        text=True,
    )
)
(PACKET / "collector.py").write_bytes(Path(__file__).read_bytes())
(PACKET / "Cargo.lock").write_bytes((ROOT / "Cargo.lock").read_bytes())
(PACKET / "manifest.json").write_text(
    json.dumps(
        {
            "source": SOURCE,
            "base": command("git", "rev-parse", BASE),
            "branch": "engineer/p04-pool-document-edits",
            "product_paths": product,
            "inputs": len(inputs),
            "reference_source": command("git", "rev-parse", "HEAD", root=REFERENCE),
            "reference_inputs": len(reference),
            "rustc": command("rustc", "-vV"),
            "cargo": command("cargo", "--version"),
            "python": sys.version,
            "uv": command("uv", "--version"),
            "checks": {
                "domain_unit": 207,
                "compile_fail_doctests": 7,
                "new_unit": 29,
                "new_compile_fail_doctests": 2,
                "all_target_clippy": "-D warnings; passed",
                "changed_path_format": "passed",
                "raw_summaries": results,
            },
            "environment": {
                "CARGO_BUILD_JOBS": "1",
                "CARGO_INCREMENTAL": "0",
                "RUST_TEST_THREADS": "1",
                "CARGO_TARGET_DIR": "/home/marc/Documents/github/lime-os/.cache/rewrite-pool-dependencies/target",
                "TMPDIR": "/home/marc/Documents/github/lime-os/.cache/tmp",
                "CCACHE_DIR": "/home/marc/Documents/github/lime-os/.cache/ccache",
                "UV_CACHE_DIR": "/home/marc/Documents/github/lime-os/.cache/uv",
            },
            "primary_references": [
                "https://man7.org/linux/man-pages/man5/fstab.5.html",
                "https://github.com/util-linux/util-linux/blob/master/lib/mangle.c",
            ],
            "acceptance": "local domain only; no installed, API/UI, runtime, package, hardware, performance or recovery acceptance",
        },
        indent=2,
    )
    + "\n"
)
hashes(
    "artifacts.sha256",
    PACKET,
    [p.name for p in PACKET.iterdir() if p.is_file() and p.name != "artifacts.sha256"],
)
print(f"Bound {SOURCE}, {len(inputs)} input files, 207 unit + 7 compile-fail checks")
