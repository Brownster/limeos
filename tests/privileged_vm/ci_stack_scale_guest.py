#!/usr/bin/env python3
"""The stack scale suite (30, 64 and 65 containers, with container-health reads through the reader) on CI's own
packages (the `stack-scale-vm` job)."""

import os

import p04_stack_scale_guest

if __name__ == "__main__":
    os.environ["LIMEOS_PACKAGE_VERSION"] = "0.4.4"
    p04_stack_scale_guest.main()
