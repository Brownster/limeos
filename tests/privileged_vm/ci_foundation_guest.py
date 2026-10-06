#!/usr/bin/env python3
"""Current payload installation and package-revision lifecycle, not a schema upgrade."""

import guest

if __name__ == "__main__":
    guest.main(package_version="0.4.4", upgrade_version="0.4.4+ci.1")
