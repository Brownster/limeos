"""Unit tests for tests/fixtures/leaf_runner.py: a harness run that tests nothing fails."""

import importlib.util
import unittest
from pathlib import Path

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


if __name__ == "__main__":
    unittest.main()
