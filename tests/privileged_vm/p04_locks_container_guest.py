#!/usr/bin/env python3
"""Repeat installed container operation recovery against schema-7 core locks."""

import hashlib
import json
import sys
from pathlib import Path

import p03_lifecycle_guest as lifecycle

if __name__ == "__main__":
    lifecycle.main(package_version="0.4.3", authority_schema=7, qualify_upgrade=False)
    output = Path(sys.argv[2])
    result = json.loads(output.read_text())
    result["package_version"] = "0.4.3"
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
        "Disposable AMD64 Debian VM; no native ARM64 or Pi/Python performance comparison",
        "Fresh standard/shadow installation and removal; no genuine upgrade in this container run",
        "Storage dependencies and effects remain pending",
    ]
    output.write_text(json.dumps(result, indent=2) + "\n")
