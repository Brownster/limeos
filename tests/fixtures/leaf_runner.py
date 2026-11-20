#!/usr/bin/env python3
"""Run `cargo test` for a prepared leaf or qualification harness and refuse a vacuous pass.

Usage: python3 tests/fixtures/leaf_runner.py MANIFEST [--expect TEST ...] [-- CARGO_TEST_ARGS]

A harness that compiles code behind a cfg its wrapper crate does not enable can
"pass" having run nothing. This runner fails, with a one-line reason, when the
build does not compile, cargo exits non-zero, any test fails, a `test result:`
line is unreadable or reports passes it cannot attribute to a named test, no
test passed, any test filter given to the test binary matched no passing test
(so one filter cannot hide another that matched nothing), or an expected test
did not pass.
"""

import argparse
import re
import subprocess
import sys
from pathlib import Path

RESULT = re.compile(
    r"^test result: (ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored; "
    r"(\d+) measured; (\d+) filtered out"
)
OUTCOME = re.compile(r"^test (.+?) \.\.\. (ok|FAILED|ignored)\b")
# libtest options that take a separate value, so the value is no filter.
VALUE_OPTIONS = {"--skip", "--test-threads", "--format", "--logfile", "--color", "-Z",
                 "--shuffle-seed"}


def summarise(output):
    """Return (passed, failed, names of passing tests) from cargo test output."""
    passed = failed = 0
    names = set()
    for line in output.splitlines():
        if match := RESULT.match(line):
            passed += int(match.group(2))
            failed += int(match.group(3))
        elif (match := OUTCOME.match(line)) and match.group(2) == "ok":
            names.add(match.group(1))
    return passed, failed, names


# cargo test options that take a separate value, so the value is no test name.
CARGO_VALUE_OPTIONS = {"-p", "--package", "--exclude", "-F", "--features", "--manifest-path",
                       "--test", "--bin", "--example", "--bench", "--target", "--target-dir",
                       "--profile", "-j", "--jobs", "--config", "-Z", "--color",
                       "--message-format", "--lockfile-path"}


def _positionals(arguments, value_options):
    found, skip_value = [], False
    for argument in arguments:
        if skip_value:
            skip_value = False
        elif argument in value_options:
            skip_value = True
        elif not argument.startswith("-"):
            found.append(argument)
    return found


def test_filters(cargo_args):
    """(filters, exact) that reach the test binary from cargo arguments.

    cargo forwards its own positional TESTNAME (before the `--`) as a filter too, so it
    is checked like the ones after it. A value of a cargo option this list does not know
    is taken for a filter and then fails for matching no test: the safe direction.
    """
    split = cargo_args.index("--") if "--" in cargo_args else len(cargo_args)
    binary = cargo_args[split + 1 :]
    filters = _positionals(cargo_args[:split], CARGO_VALUE_OPTIONS)
    filters += _positionals(binary, VALUE_OPTIONS)
    return filters, "--exact" in binary


def verdict(returncode, output, expected, filters=(), exact=False):
    """None when the run counts as a real pass, else the one-line reason it does not."""
    lines = output.splitlines()
    if any(line.startswith("error: could not compile") for line in lines):
        first = next(line for line in lines if line.startswith("error"))
        return f"the test build did not compile: {first.strip()}"
    passing, failing, passed, failed = [], [], 0, 0
    for line in lines:
        if line.startswith("test result:"):
            match = RESULT.match(line)
            if not match:
                return f"unrecognised result line: {line.strip()!r}"
            passed += int(match.group(2))
            failed += int(match.group(3))
            if match.group(1) != "ok":
                failed = max(failed, 1)
        elif match := OUTCOME.match(line):
            if match.group(2) == "ok":
                passing.append(match.group(1))
            elif match.group(2) == "FAILED":
                failing.append(match.group(1))
    if failing or failed:
        return f"cargo test failed: {max(failed, len(failing))} failed {sorted(failing)}"
    if returncode != 0:
        return f"cargo test failed (exit {returncode})"
    if passed != len(passing):
        return f"result lines report {passed} passed but {len(passing)} passing tests are named"
    if passed == 0:
        return "no tests ran; the harness may be compiling its subject out"

    def matches(name, test_filter):
        return name == test_filter if exact else test_filter in name

    empty = [f for f in filters if not any(matches(name, f) for name in passing)]
    if empty:
        return f"no test passed for filter {empty[0]!r} (of {len(filters)}); it may be compiled out"
    missing = sorted(
        name for name in expected
        if not any(n == name or n.endswith("::" + name) for n in passing)
    )
    if missing:
        return f"expected tests did not pass: {missing}"
    return None


ROOT = Path(__file__).resolve().parents[2]


def run_deferred(groups):
    """Run each (package, target arguments, test filters) group in the workspace
    with `--features deferred`; every filter of every group must pass a test."""
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
    filters, exact = test_filters(extra)
    reason = verdict(process.returncode, process.stdout, args.expect, filters, exact)
    passed = summarise(process.stdout)[0]
    if reason:
        print(f"leaf_runner: FAIL: {reason}", file=sys.stderr)
        return 1
    print(f"leaf_runner: {passed} tests passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
