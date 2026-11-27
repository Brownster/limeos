#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.11"
# dependencies = ["playwright==1.62.0"]
# ///
"""Storage page against the closed household fixture; no live host.

Checks the summary band, disks with their partitions nested, the root
filesystem's usage bar (and no invented bars elsewhere), pools, the
administrator's inventory details behind each disk's disclosure (read once
per visit and on manual refresh), a refused or failed inventory read shown
without an error, the viewer making no storage reads, and 390 px. With
--screens DIR, also writes the screenshots.
"""

import argparse
import json
import sys
from pathlib import Path

from playwright.sync_api import sync_playwright

sys.path.insert(0, str(Path(__file__).resolve().parent))
from household_fixture import Household  # noqa: E402
from stack_cards import no_overflow, settle, shot, small_targets, watch  # noqa: E402

HOST = "limeos-storage.invalid"
BASE = f"https://{HOST}"
INVENTORY = "/api/v1/storage/inventory"


def reads(fixture):
    return sum(1 for path, _ in fixture.calls if path == INVENTORY)


def open_storage(browser, problems, role="administrator", inventory="ok", width=1440, height=900):
    page = browser.new_page(viewport={"width": width, "height": height}, has_touch=width < 600)
    watch(page, problems)
    fixture = Household(HOST, role=role)
    fixture.storage_inventory = inventory
    page.route(f"{BASE}/**", fixture.respond)
    page.goto(f"{BASE}/storage")
    page.locator('article[aria-label="Disk sda"]').wait_for()
    settle(page)
    return page, fixture


def admin_checks(browser, checks, problems, screens):
    page, fixture = open_storage(browser, problems)
    band = page.get_by_role("group", name="Storage summary")
    text = band.inner_text()
    for fragment in ("DISKS", "4", "7.1 TB in total", "4 of 5", "1 not mounted", "POOLS", "78.9%"):
        assert fragment in text, (fragment, text)
    checks.append("summary band: disks, total capacity, filesystems mounted, pools, root use")

    sd = page.locator('article[aria-label="Disk mmcblk0"]')
    assert "mmcblk0p1" in sd.inner_text() and "/boot/firmware" in sd.inner_text() and "vfat" in sd.inner_text()
    assert "In use" in sd.inner_text() and "Serial 0x8d3e2f1a" in sd.inner_text()
    sdc = page.locator('article[aria-label="Disk sdc"]')
    assert "Not mounted" in sdc.inner_text() and "sdc1" in sdc.inner_text()
    bars = page.get_by_role("progressbar")
    names = [bars.nth(i).get_attribute("aria-label") for i in range(bars.count())]
    assert names == ["Root filesystem used", "mmcblk0p2 used"], names
    assert sd.get_by_role("progressbar", name="mmcblk0p2 used").get_attribute("aria-valuetext").startswith("78.9% used")
    pool = page.locator('article[aria-label="Pool /mnt/media"]')
    assert "mergerfs pool" in pool.inner_text() and "Mounted" in pool.inner_text()
    checks.append("disks with nested partitions, status, filesystem and mount; usage bar only on root; pool")
    shot(page, screens, "storage-1440.png", full_page=True)

    # Administrator inventory: serials, UUIDs and fstab, read once per visit.
    assert reads(fixture) == 1, fixture.calls
    sda = page.locator('article[aria-label="Disk sda"]')
    sda.locator("summary").click()
    details = sda.locator("details").inner_text()
    for fragment in ("WD-WX12A3456789", "a1b2c3d4-0e5f-4a6b-9c7d-8e9f0a1b2c3d", "Mounted at boot by /etc/fstab (/mnt/storage)", "/srv/media", "read/write"):
        assert fragment in details, (fragment, details)
    assert sda.get_by_role("button", name="Copy sda1 filesystem UUID").count() == 1
    sdb = page.locator('article[aria-label="Disk sdb"]')
    sdb.locator("summary").click()
    assert "LimeOS's managed fstab entry" in sdb.locator("details").inner_text()
    sdc.locator("summary").click()
    assert "Not in /etc/fstab" in sdc.locator("details").inner_text()
    sda.scroll_into_view_if_needed()
    shot(page, screens, "storage-disk-expanded-1440.png", full_page=True)
    page.wait_for_timeout(1500)
    assert reads(fixture) == 1, "no inventory polling"
    page.get_by_role("button", name="Refresh now").click()
    settle(page)
    assert reads(fixture) == 2, "a manual refresh reads the inventory again"
    checks.append("inventory: serials, UUIDs, fstab persistence and every mount; one read per visit and per manual refresh")
    page.close()

    # Refused (no storage_manage grant) and failed reads: basic view, no error.
    for mode, note in (("denied", "shown to administrators who manage storage"), ("down", "storage reader didn't answer")):
        page, fixture = open_storage(browser, problems, inventory=mode)
        assert page.locator('[role="alert"]').count() == 0, mode
        sda = page.locator('article[aria-label="Disk sda"]')
        assert "Serial WD-WX12A3456789" in sda.inner_text(), "the serial in the disk's identity still shows"
        sda.locator("summary").click()
        assert note in sda.locator("details").inner_text(), mode
        if mode == "denied":
            page.get_by_role("button", name="Refresh now").click()
            settle(page)
            assert reads(fixture) == 1, "a refusal is not retried during the visit"
        page.close()
    checks.append("inventory refused or unavailable: basic view, a quiet note, no alert; refusal not retried")


def viewer_checks(browser, checks, problems):
    page, fixture = open_storage(browser, problems, role="viewer", width=1100, height=850)
    assert page.locator("#mount-heading").count() == 0
    assert "mmcblk0p2" in page.locator('article[aria-label="Disk mmcblk0"]').inner_text()
    assert not any(path.startswith("/api/v1/storage/") for path, _ in fixture.calls), fixture.calls
    page.locator('article[aria-label="Disk sda"] summary').click()
    assert "a1b2c3d4-0e5f-4a6b-9c7d-8e9f0a1b2c3d" in page.locator('article[aria-label="Disk sda"] details').inner_text()
    checks.append("viewer: disks and identifiers from the overview, no storage reads, no mount controls")
    page.close()


def mobile_checks(browser, checks, problems, screens):
    page, _ = open_storage(browser, problems, width=390, height=844)
    assert no_overflow(page)
    small = small_targets(page)
    assert not small, small
    shot(page, screens, "storage-390.png", full_page=True)
    page.locator('article[aria-label="Disk sda"] summary').click()
    assert no_overflow(page)
    shot(page, screens, "storage-disk-expanded-390.png", full_page=True)
    checks.append("390 px: cards, no overflow (also with a disclosure open), 44 px targets")
    page.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--screens", type=Path)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    if args.screens:
        args.screens.mkdir(parents=True, exist_ok=True)
    checks, problems = [], []
    with sync_playwright() as playwright:
        browser = playwright.chromium.launch(
            executable_path="/usr/bin/google-chrome",
            headless=True,
            args=["--disable-dev-shm-usage"],
        )
        try:
            admin_checks(browser, checks, problems, args.screens)
            viewer_checks(browser, checks, problems)
            mobile_checks(browser, checks, problems, args.screens)
            assert not problems, problems
        finally:
            browser.close()
    (args.output / "result.json").write_text(
        json.dumps({"checks": checks, "live_host_operations": 0}, indent=2) + "\n"
    )
    print(f"Storage page browser checks passed ({len(checks)}).")


if __name__ == "__main__":
    main()
