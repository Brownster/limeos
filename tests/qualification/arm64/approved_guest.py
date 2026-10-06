#!/usr/bin/env python3
"""Run current approved storage/container guest suites with frozen ARM64 identity.

Storage uses only the four serial/size/empty guarded fixture disks. Both suites
retain their existing failure assertions. Results include failures and fixture
identity. The storage suite also exercises actual apt replacement and removal.
"""

import argparse
import json
import shutil
import subprocess
import sys
import time
from pathlib import Path

from qualification import guest_guard, installed_binaries, load_build, sha, verify_files


def teardown(version):
    def run(*command, check=True):
        return subprocess.run(
            command, check=check, text=True, capture_output=True, timeout=180
        )

    def active():
        return (
            run(
                "systemctl", "is-active", "limeos-storage-targets", check=False
            ).returncode
            == 0
        )

    # The shared suite has already tested shadow removal with an active standard
    # target service. These additional checks call apt, including real removal.
    run("systemctl", "start", "limeos-storage-targets")
    assert active()
    replacement = run("apt-get", "install", "-y", "--reinstall", "limeos=" + version)
    assert not active(), "replacement left target service running"
    run("systemctl", "start", "limeos-storage-targets")
    assert active()
    removal = run("apt-get", "remove", "-y", "limeos")
    assert not active(), "standard removal left target service running"
    run("apt-get", "install", "-y", "limeos=" + version)
    assert not active(), "configure enabled optional target service"
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline:
        if run("/usr/lib/limeos/limeosctl", "status", check=False).returncode == 0:
            break
        time.sleep(0.1)
    else:
        raise AssertionError("core not ready after actual standard removal/reinstall")
    return {
        "replacement": replacement.stdout,
        "removal": removal.stdout,
        "target_service_dormant_after_reinstall": True,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--suite", choices=["storage", "container"], required=True)
    parser.add_argument("--repo", type=Path, required=True)
    parser.add_argument("--expected", type=Path, required=True)
    parser.add_argument("--hashes", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    guest_guard()
    if args.output.exists():
        parser.error("output exists; select a new result file")
    build = load_build(args.expected, args.repo)
    if build["identity"]["authority_schema"] != 8:
        parser.error("approved-operation fixtures require schema 8")
    fixtures = Path(__file__).resolve().parents[3]
    verify_files(fixtures, build["fixtures_sha256"])
    sys.path.insert(0, str(fixtures / "tests/privileged_vm"))
    if args.hashes.resolve() != Path("/root/werkzeug-hashes.json"):
        shutil.copyfile(args.hashes, "/root/werkzeug-hashes.json")
    result = {
        "identity": build["identity"],
        "build_result_sha256": sha(args.expected),
        "suite": args.suite,
        "started": time.time(),
        "summary": "incomplete",
    }
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    raw = args.output.with_name(args.output.stem + "-fixture.json")
    if raw.exists():
        parser.error("fixture output exists; select a new result file")
    sys.argv = [__file__, str(args.repo), str(raw)]
    try:
        if args.suite == "storage":
            import p04_approved_guest as fixture

            fixture.main(
                package_version=build["identity"]["package_version"],
                qualify_upgrade=False,
            )
            result["package_teardown"] = teardown(build["identity"]["package_version"])
        else:
            import p04_approved_container_guest as fixture

            fixture.main(package_version=build["identity"]["package_version"])
        result["installed_sha256"] = installed_binaries(build)
        result["fixture_result"] = json.loads(raw.read_text())
        result["summary"] = "pass"
    except Exception as error:
        result.update(summary="fail", error=repr(error))
        if raw.exists():
            result["fixture_result"] = json.loads(raw.read_text())
        raise
    finally:
        result["finished"] = time.time()
        args.output.write_text(json.dumps(result, indent=2) + "\n")


if __name__ == "__main__":
    main()
