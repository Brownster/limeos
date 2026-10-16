#!/usr/bin/env python3
"""Current reference-layout suite; genuine historical migrations run separately."""

import p03_reference_guest
from current_authority_schema import current_authority_schema

if __name__ == "__main__":
    p03_reference_guest.main(
        package_version="0.4.4",
        authority_schema=current_authority_schema(),
        qualify_upgrade=False,
    )
