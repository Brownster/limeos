"""Unit tests for tests/fixtures/leaf_runner.py: a harness run that tests nothing fails."""

import contextlib
import importlib.util
import io
import subprocess
import unittest
from pathlib import Path
from unittest import mock

SCRIPT = Path(__file__).resolve().parents[1] / "fixtures/leaf_runner.py"
spec = importlib.util.spec_from_file_location("leaf_runner", SCRIPT)
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)

PASS = """running 2 tests
test deployment::catalog::tests::plans ... ok
test deployment::catalog::tests::refuses ... ok

test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 5 filtered out
   Doc-tests limeos_domain
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
"""
EMPTY = "test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 49 filtered out\n"


class VerdictTests(unittest.TestCase):
    def test_real_pass(self):
        self.assertIsNone(runner.verdict(0, PASS, []))
        self.assertEqual(runner.summarise(PASS)[:2], (2, 0))

    def test_zero_tests_is_a_failure(self):
        self.assertIn("no tests ran", runner.verdict(0, EMPTY, []))
        self.assertIn("no tests ran", runner.verdict(0, "", []))

    def test_cargo_failure(self):
        self.assertIn("cargo test failed", runner.verdict(101, PASS, []))
        failed = PASS.replace("2 passed; 0 failed", "1 passed; 1 failed")
        self.assertIn("cargo test failed", runner.verdict(0, failed, []))

    def test_expected_names(self):
        self.assertIsNone(runner.verdict(0, PASS, ["plans", "catalog::tests::refuses"]))
        self.assertIn("missing", runner.verdict(0, PASS, ["missing"]))
        # A suffix must match whole path segments.
        self.assertIn("ans", runner.verdict(0, PASS, ["ans"]))


def output(*lines):
    return "\n".join(lines) + "\n"


MEDIA_ONLY = output(
    "running 2 tests",
    "test deployment::media::tests::plans ... ok",
    "test deployment::media::tests::refuses ... ok",
    "",
    "test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 40 filtered out",
)
ACTIVITY_IGNORED = output(
    "running 3 tests",
    "test deployment::activity::tests::live ... ignored, needs a host",
    "test deployment::media::tests::plans ... ok",
    "test deployment::media::tests::refuses ... ok",
    "",
    "test result: ok. 2 passed; 0 failed; 1 ignored; 0 measured; 40 filtered out",
)
COMPILE_ERROR = output(
    "   Compiling limeos-domain v0.2.0",
    "error[E0425]: cannot find value `x` in this scope",
    "error: could not compile `limeos-domain` (lib test) due to 1 previous error",
)
ACTIVITY = ["deployment::activity", "deployment::media"]


class RunnerTests(unittest.TestCase):
    """Drive run_deferred and main with mocked cargo output."""

    def run_groups(self, stdout, returncode=0, filters=ACTIVITY):
        completed = subprocess.CompletedProcess([], returncode, stdout=stdout)
        out, err = io.StringIO(), io.StringIO()
        with mock.patch.object(runner.subprocess, "run", return_value=completed) as run, \
                contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            status = runner.run_deferred([("limeos-domain", ["--lib"], filters)])
        return status, err.getvalue(), run

    def assert_fails(self, stdout, reason, returncode=0, filters=ACTIVITY):
        status, err, _ = self.run_groups(stdout, returncode, filters)
        self.assertEqual(status, 1, err)
        lines = err.splitlines()
        self.assertEqual(len(lines), 1, err)
        self.assertTrue(lines[0].startswith("leaf_runner: FAIL: "), err)
        self.assertIn(reason, lines[0])

    def test_each_filter_must_pass_a_test(self):
        # The activity harness: media passes, activity matches nothing.
        self.assert_fails(MEDIA_ONLY, "'deployment::activity'")

    def test_ignored_only_filter_fails(self):
        self.assert_fails(ACTIVITY_IGNORED, "'deployment::activity'")

    def test_failed_line_fails_even_with_a_clean_summary(self):
        lost = MEDIA_ONLY.replace(
            "test deployment::media::tests::refuses ... ok",
            "test deployment::activity::tests::reads ... FAILED",
        )
        self.assert_fails(lost, "deployment::activity::tests::reads")

    def test_unattributed_passes_fail(self):
        # Summary claims more passes than the runner can attribute to names.
        claimed = MEDIA_ONLY.replace("2 passed", "3 passed")
        self.assert_fails(claimed, "3 passed")

    def test_unrecognised_result_line_fails(self):
        garbled = MEDIA_ONLY + "test result: ok, 5 passed\n"
        self.assert_fails(garbled, "unrecognised", filters=["deployment::media"])

    def test_compile_failure_is_named(self):
        self.assert_fails(COMPILE_ERROR, "did not compile", returncode=101)
        self.assertIn("error[E0425]", runner.verdict(101, COMPILE_ERROR, []))
        # A failing test makes cargo print `error: test failed`, which is no build error.
        failing = MEDIA_ONLY.replace("... ok", "... FAILED").replace("ok. 2 passed; 0", "FAILED. 0 passed; 2")
        reason = runner.verdict(101, failing + "error: test failed, to rerun pass `--lib`\n", [])
        self.assertIn("2 failed", reason)

    def test_other_failures_are_one_line(self):
        self.assert_fails(MEDIA_ONLY, "exit 101", returncode=101, filters=["deployment::media"])
        self.assert_fails(EMPTY, "no tests ran", filters=[])
        self.assert_fails("", "no tests ran", filters=[])

    def test_passing_groups(self):
        both = MEDIA_ONLY.replace(
            "test deployment::media::tests::refuses ... ok",
            "test deployment::activity::tests::reads ... ok",
        )
        status, err, run = self.run_groups(both)
        self.assertEqual(status, 0, err)
        command = run.call_args.args[0]
        self.assertEqual(command[-3:], ["--", *ACTIVITY])
        self.assertIn("--features", command)

    def test_filters_are_read_from_test_binary_arguments(self):
        self.assertEqual(
            runner.test_filters(["-p", "x", "--", "a::", "--skip", "slow", "--test-threads", "2",
                                 "--format=terse", "b"]),
            (["a::", "b"], False),
        )
        self.assertEqual(runner.test_filters(["--lib"]), ([], False))
        self.assertEqual(runner.test_filters(["--", "--exact", "a::b"]), (["a::b"], True))
        # Under --exact a filter must equal a test name.
        self.assertIn("'deployment::media'", runner.verdict(
            0, MEDIA_ONLY, [], filters=["deployment::media"], exact=True))
        self.assertIsNone(runner.verdict(
            0, MEDIA_ONLY, [], filters=["deployment::media::tests::plans"], exact=True))

    def test_every_harness_filters_each_group(self):
        """Each harness group names filters, so the per-filter rule covers it."""
        fixtures = SCRIPT.parent
        harnesses = [p for p in fixtures.rglob("check-lea*.py") if "run_deferred" in p.read_text()]
        self.assertGreater(len(harnesses), 10)
        for path in harnesses:
            spec = importlib.util.spec_from_file_location(path.stem.replace("-", "_"), path)
            module = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(module)
            for package, _targets, filters in module.GROUPS:
                self.assertTrue(filters, f"{path}: {package} has no filter")


if __name__ == "__main__":
    unittest.main()
