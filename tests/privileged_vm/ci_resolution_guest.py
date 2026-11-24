#!/usr/bin/env python3
"""The needs-intervention resolution suite on CI's own packages (the non-gating `resolution-vm` job)."""

import p04_resolution_guest

if __name__ == "__main__":
    p04_resolution_guest.main(package_version="0.4.4")
