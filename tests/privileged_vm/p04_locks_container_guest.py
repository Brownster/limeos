#!/usr/bin/env python3
"""Repeat installed container operation recovery against schema-7 core locks."""

import p03_lifecycle_guest as lifecycle

if __name__ == "__main__":
    lifecycle.main(package_version="0.4.3", authority_schema=7, qualify_upgrade=False)
