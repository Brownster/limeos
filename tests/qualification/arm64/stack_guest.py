#!/usr/bin/env python3
"""Run the whole-stack lifecycle or scale fixture in a fresh native ARM64 guest.

A thin wrapper in the pattern of `approved_guest.py`: it validates the build
record and the bundled fixture bytes, then calls the unchanged
`tests/privileged_vm/p04_stack_lifecycle_guest.py` or `p04_stack_scale_guest.py`
with the candidate's package version and the signed repository. Failures stay in
the result JSON. The fixtures refuse to run outside the disposable
`limeos-p01-test` guest; `guest_guard` additionally requires native aarch64 KVM.
"""

import argparse
import json
import os
import shutil
import sys
import time
from pathlib import Path

from qualification import guest_guard, installed_binaries, load_build, sha, verify_files


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--suite", choices=["lifecycle", "scale"], required=True)
    parser.add_argument("--repo", type=Path, required=True)
    parser.add_argument("--expected", type=Path, required=True)
    parser.add_argument("--hashes", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    guest_guard()
    if args.output.exists():
        parser.error("output exists; select a new result file")
    build = load_build(args.expected, args.repo)
    version = build["identity"]["package_version"]
    fixtures = Path(__file__).resolve().parents[3]
    verify_files(fixtures, build["fixtures_sha256"])
    sys.path.insert(0, str(fixtures / "tests/privileged_vm"))
    if args.hashes and args.hashes.resolve() != Path("/root/werkzeug-hashes.json"):
        shutil.copyfile(args.hashes, "/root/werkzeug-hashes.json")
    result = {
        "identity": build["identity"],
        "build_result_sha256": sha(args.expected),
        "suite": "stack-" + args.suite,
        "started": time.time(),
        "summary": "incomplete",
    }
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    raw = args.output.with_name(args.output.stem + "-fixture.json")
    if raw.exists():
        parser.error("fixture output exists; select a new result file")
    # Both fixtures read `<repository> <output>` from argv.
    sys.argv = [__file__, str(args.repo), str(raw)]
    try:
        if args.suite == "lifecycle":
            import p04_stack_lifecycle_guest as fixture

            fixture.main(package_version=version)
        else:
            import p04_stack_scale_guest as fixture

            os.environ["LIMEOS_PACKAGE_VERSION"] = version
            fixture.main()
        result["installed_sha256"] = installed_binaries(build)
        result["fixture_result"] = json.loads(raw.read_text())
        result["summary"] = "pass"
    except BaseException as error:
        result.update(summary="fail", error=repr(error))
        if raw.exists():
            result["fixture_result"] = json.loads(raw.read_text())
        raise
    finally:
        result["finished"] = time.time()
        args.output.write_text(json.dumps(result, indent=2) + "\n")


if __name__ == "__main__":
    main()
