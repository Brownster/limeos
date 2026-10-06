#!/usr/bin/env python3
"""Installed lifecycle and interruption regression for authority schema 8."""

import hashlib
import json
import sys
from pathlib import Path

import p03_lifecycle_guest as lifecycle

if __name__ == "__main__":
    lifecycle.main(package_version="0.4.4", authority_schema=8, qualify_upgrade=False)
    output = Path(sys.argv[2])
    result = json.loads(output.read_text())
    result["package_version"] = "0.4.4"
    result["binary_sha256"] = {
        name: hashlib.sha256((Path("/usr/lib/limeos") / name).read_bytes()).hexdigest()
        for name in [
            "limeos-core",
            "limeos-executor",
            "limeosctl",
            "limeos-password-worker",
        ]
    }
    result["limitations"] = [
        "Disposable AMD64 VM; current fresh standard/shadow lifecycle, no genuine upgrade in this run",
        "Live container storage dependency discovery and mount/fstab effects remain pending",
    ]
    output.write_text(json.dumps(result, indent=2) + "\n")
