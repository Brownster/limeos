#!/usr/bin/env python3
"""Run the network group planner and adapter tests in place.

This harness once copied the modules into a separate wrapper crate because
they were not declared in the workspace. They are now declared behind the
`deferred` feature (docs/deferred-code.md), so it runs their tests in the
workspace with that feature, through leaf_runner, which fails when no test ran.
Earlier evidence that names this script reproduces only at its recorded source.

Usage: python3 tests/fixtures/deployment/networks/check-leaves.py
"""

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
from leaf_runner import run_deferred  # noqa: E402

GROUPS = [
    ('limeos-domain', ['--lib'], ['deployment::networks']),
    ('limeos-executor-container', ['--lib'], ['deployment::networks']),
]

if __name__ == "__main__":
    sys.exit(run_deferred(GROUPS))
