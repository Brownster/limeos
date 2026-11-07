#!/usr/bin/env python3
"""Run the real guest_supervisor.py with one change: the absolute QEMU path.

The supervisor insists on `/usr/bin/qemu-system-aarch64`, which this workstation does
not have (and must not). Everything else - the argv checks, pidfd binding, deadline
handling, owner.json/process.json - is the unmodified production file. Only the
exec of that one path is redirected to the rehearsal stub.

    python3 -I supervisor_runner.py <path/to/guest_supervisor.py> launch|stop|wait ...
"""

import os
import runpy
import subprocess
import sys

script = sys.argv[1]
sys.argv = sys.argv[1:]
stub = os.path.join(os.environ["LIMEOS_REHEARSAL_CTL"], "bin/qemu-system-aarch64")
original = subprocess.Popen


class Redirected(original):
    def __init__(self, argv, *args, **kwargs):
        argv = [stub if a == "/usr/bin/qemu-system-aarch64" else a for a in argv]
        super().__init__(argv, *args, **kwargs)


subprocess.Popen = Redirected
runpy.run_path(script, run_name="__main__")
