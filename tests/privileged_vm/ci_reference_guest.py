#!/usr/bin/env python3
"""Current reference-layout suite; genuine historical migrations run separately."""

import p03_reference_guest

if __name__ == "__main__":
    p03_reference_guest.main(
        package_version="0.4.4", authority_schema=8, qualify_upgrade=False
    )
