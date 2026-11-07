#!/usr/bin/env python3
"""Run the real run_pi_night.main() against the rehearsal host.

Replaced for the rehearsal, and nothing else:
  * the candidate record check (the rehearsal candidate is synthetic) unless
    LIMEOS_REHEARSAL_REAL_INPUTS is set, and the pinned Debian image digest, which is
    set to the digest of the synthetic image;
  * Host.run's per-call timeouts are multiplied by LIMEOS_REHEARSAL_TIMEOUT_SCALE
    (floor 3 s) so hang scenarios finish in seconds.

It refuses any --host other than the rehearsal host, so it cannot be pointed at wybie.
"""

import hashlib
import json
import os
import sys
from pathlib import Path

ARM64 = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ARM64))
import native_vm  # noqa: E402
import qualification  # noqa: E402
import run_pi_night as night  # noqa: E402


def host_argument(argv):
    for index, value in enumerate(argv):
        if value == "--host":
            return argv[index + 1]
        if value.startswith("--host="):
            return value.split("=", 1)[1]
    return None


def main(argv):
    host = host_argument(argv)
    rehearsal_host = os.environ.get("LIMEOS_REHEARSAL_HOST")
    if (
        not host
        or host != rehearsal_host
        or native_vm.production_host(host, resolve=False)
    ):
        sys.exit("launch.py only drives the rehearsal host, never a real one")
    image = Path(os.environ["LIMEOS_REHEARSAL_IMAGE"])
    night.IMAGE_SHA512 = hashlib.sha512(image.read_bytes()).hexdigest()
    if not os.environ.get("LIMEOS_REHEARSAL_REAL_INPUTS"):
        qualification.load_build = lambda path, repo=None: json.loads(Path(path).read_text())
    scale = float(os.environ.get("LIMEOS_REHEARSAL_TIMEOUT_SCALE", "1"))
    original = night.Host.run

    def scaled(self, command, timeout=60, input=None):
        return original(self, command, timeout=max(3.0, timeout * scale), input=input)

    night.Host.run = scaled
    return night.main(argv)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
