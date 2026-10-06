#!/usr/bin/env python3
"""Qualify current CI-labelled payloads; these are not historical releases."""

import p04_dependencies_guest

if __name__ == "__main__":
    p04_dependencies_guest.main(package_version="0.4.4")
