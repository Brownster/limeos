#!/usr/bin/env python3
"""Run `cargo test` for a prepared leaf or qualification harness and refuse a vacuous pass.

Usage: python3 tests/fixtures/leaf_runner.py MANIFEST [--expect TEST ...] [-- CARGO_TEST_ARGS]

A harness that compiles code behind a cfg its wrapper crate does not enable can
"pass" having run nothing. This runner sums every `test result:` line and fails
when the total is zero, when cargo fails, or when an expected test did not pass.
"""

import argparse
import re
import subprocess
import sys
from pathlib import Path

RESULT = re.compile(r"^test result: \w+\. (\d+) passed; (\d+) failed")
PASSED = re.compile(r"^test (\S+) \.\.\. ok$")


def summarise(output):
    """Return (passed, failed, names of passing tests) from cargo test output."""
    passed = failed = 0
    names = set()
    for line in output.splitlines():
        if match := RESULT.match(line):
            passed += int(match.group(1))
            failed += int(match.group(2))
        elif match := PASSED.match(line):
            names.add(match.group(1))
    return passed, failed, names


def verdict(returncode, output, expected):
    """None when the run counts as a real pass, else the reason it does not."""
    passed, failed, names = summarise(output)
    if returncode != 0 or failed:
        return f"cargo test failed (exit {returncode}, {failed} failed)"
    if passed == 0:
        return "no tests ran; the harness may be compiling its subject out"
    missing = sorted(
        name for name in expected if not any(n == name or n.endswith("::" + name) for n in names)
    )
    if missing:
        return f"expected tests did not pass: {missing}"
    return None


ROOT = Path(__file__).resolve().parents[2]


def run_deferred(groups):
    """Run each (package, target arguments, test filters) group in the workspace
    with `--features deferred`; every group must pass at least one test."""
    status = 0
    for package, targets, filters in groups:
        status |= main(
            [str(ROOT / "Cargo.toml"), "--", "--locked", "-p", package, "--features", "deferred",
             *targets, "--", *filters]
        )
    return status


def main(argv=None):
    argv = list(sys.argv[1:] if argv is None else argv)
    extra = []
    if "--" in argv:
        index = argv.index("--")
        argv, extra = argv[:index], argv[index + 1 :]
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("manifest")
    parser.add_argument("--expect", action="append", default=[])
    args = parser.parse_args(argv)
    command = ["cargo", "test", "--offline", "--manifest-path", args.manifest, *extra]
    print("+", " ".join(command), flush=True)
    process = subprocess.run(command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    sys.stdout.write(process.stdout)
    reason = verdict(process.returncode, process.stdout, args.expect)
    passed = summarise(process.stdout)[0]
    if reason:
        print(f"leaf_runner: FAIL: {reason}", file=sys.stderr)
        return 1
    print(f"leaf_runner: {passed} tests passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
