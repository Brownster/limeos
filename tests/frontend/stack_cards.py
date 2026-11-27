#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.11"
# dependencies = ["playwright==1.62.0"]
# ///
"""Stacks page cards against the closed household fixture; no live host.

Checks the card per stack (status, running-of-total, containers by name with
links to the Containers page), the technical disclosure, the whole-stack
plan -> approve -> queue flow with the job shown on its card, the viewer's
read-only view, the stack reader being down after a reboot, a failed read,
and 390 px. With --screens DIR, also writes the screenshots.
"""

import argparse
import json
import sys
from pathlib import Path

from playwright.sync_api import sync_playwright

sys.path.insert(0, str(Path(__file__).resolve().parent))
from household_fixture import Household  # noqa: E402

HOST = "limeos-stack-cards.invalid"
BASE = f"https://{HOST}"


def watch(page, problems):
    page.on("pageerror", lambda error: problems.append(f"pageerror: {error}"))
    page.on(
        "console",
        lambda message: problems.append(f"console: {message.text}")
        if message.type == "error" and "Failed to load resource" not in message.text
        else None,
    )


def settle(page):
    page.wait_for_load_state("networkidle")
    page.wait_for_timeout(250)


def shot(page, screens, name, **options):
    if screens:
        page.screenshot(path=str(screens / name), **options)


def no_overflow(page):
    return page.evaluate("document.documentElement.scrollWidth <= window.innerWidth + 1")


def small_targets(page):
    return page.evaluate(
        """() => [...document.querySelectorAll('main button, main a, header button')]
          .filter(e => e.getClientRects().length && !e.closest('table, .state-support, .link-list, p'))
          .map(e => [e.textContent.trim() || e.getAttribute('aria-label'), Math.round(e.getBoundingClientRect().height)])
          .filter(([, h]) => h < 44)"""
    )


def admin_checks(browser, checks, problems, screens):
    page = browser.new_page(viewport={"width": 1440, "height": 900})
    watch(page, problems)
    fixture = Household(HOST)
    page.route(f"{BASE}/**", fixture.respond)
    page.goto(f"{BASE}/stacks")
    panel = page.locator('section[aria-labelledby="stack-management-heading"]')
    media = page.locator('article[aria-label="Stack media"]')
    downloads = page.locator('article[aria-label="Stack downloads"]')
    media.wait_for()
    settle(page)

    # Status, counts and containers by name, not by ID.
    assert "Partly running" in media.inner_text()
    assert "3 of 4 service containers running" in media.inner_text()
    assert "Running" in downloads.inner_text() and "1 unhealthy" in downloads.inner_text()
    for name in ("jellyfin", "sonarr", "radarr", "lidarr"):
        assert name in media.inner_text(), name
    assert "exited" in media.inner_text()
    visible = media.inner_text()
    assert "container:" not in visible and "sha256:" not in visible and "000000000001" not in visible, visible
    link = media.get_by_role("link", name="jellyfin", exact=False)
    assert link.get_attribute("href") == "/containers?search=jellyfin"
    bar = media.get_by_role("progressbar", name="media: service containers running")
    assert bar.get_attribute("aria-valuetext") == "3 of 4 running"
    summary = page.get_by_role("group", name="Stacks summary")
    assert "5 / 6" in summary.inner_text() and "1 unhealthy" in summary.inner_text()
    refresh = panel.get_by_role("button", name="Refresh stacks", exact=True)
    assert refresh.evaluate("e => e.classList.contains('quiet')"), "Refresh stacks should be a quiet button"
    checks.append("cards: status, running of total, containers by name with Containers links, quiet refresh")
    shot(page, screens, "stacks-1440.png", full_page=True)

    # Technical details: full IDs, image digests and files behind a disclosure.
    media.locator("summary").filter(has_text="Services and members").click()
    table = media.locator("table")
    table.wait_for()
    assert table.locator("tbody tr").count() == 4
    details = media.locator("details").inner_text()
    assert f"{1:064x}" in details and "sha256:" + "1" * 64 in details
    assert "/opt/stacks/media/compose.yaml" in details
    copies = media.get_by_role("button", name="Copy jellyfin container ID")
    assert copies.count() == 1, "copy button for the container ID"
    checks.append("disclosure: full container IDs, image digests, Compose file, copy buttons")
    media.scroll_into_view_if_needed()
    shot(page, screens, "stacks-expanded-1440.png", full_page=True)
    media.locator("summary").filter(has_text="Services and members").click()

    # Whole-stack restart: plan -> approve -> queue, then the job on its card.
    media.get_by_role("button", name="Restart stack", exact=True).click()
    dialog = page.get_by_role("dialog", name="Restart stack media")
    dialog.wait_for()
    dialog.get_by_role("button", name="Approve and restart stack", exact=True).wait_for()
    targets = dialog.locator(".plan-targets").inner_text()
    for name in ("jellyfin", "sonarr", "radarr"):
        assert name in targets, targets
    assert "lidarr" not in targets, "restart acts only on running containers"
    shot(page, screens, "stacks-approve-1440.png")
    dialog.get_by_role("button", name="Approve and restart stack", exact=True).click()
    dialog.wait_for(state="hidden")
    strip = media.locator(".stack-job")
    strip.get_by_text("Restarting stack · in progress").wait_for()
    assert strip.inner_text().count("waiting") == 3, strip.inner_text()
    assert media.get_by_role("button", name="Restart stack", exact=True).is_disabled()
    fixture.job_receipt = True
    strip.get_by_text("done").wait_for(timeout=10_000)
    assert "jellyfin" in strip.inner_text()
    media.scroll_into_view_if_needed()
    shot(page, screens, "stacks-job-1440.png", full_page=True)
    fixture.job_state = "succeeded"
    strip.get_by_text("Restarted stack").wait_for(timeout=10_000)
    assert strip.inner_text().count("done") == 3, strip.inner_text()
    assert "Recent stack operations" in panel.inner_text()
    posts = [p for p, m in fixture.calls if m == "POST"]
    assert posts == ["/api/v1/stack/plans", "/api/v1/stack/plans/" + f"{1:064x}" + "/approval", "/api/v1/stack/jobs"], posts
    checks.append("restart: plan, approve and queue; job on its card with per-container outcomes; buttons held")

    # After a host reboot the stack reader is not running: say so plainly.
    fixture.stacks_mode = "reader_down"
    refresh.click()
    panel.get_by_text("The stack reader isn't running").wait_for()
    assert "systemctl start limeos-storage-reader" in panel.inner_text()
    observed = page.locator('article[aria-label="Observed stack media"]')
    observed.wait_for()
    assert "jellyfin" in observed.inner_text() and "lidarr" in observed.inner_text()
    assert page.get_by_role("group", name="Whole-stack actions for media").count() == 0
    assert page.locator('[role="alert"]').count() == 0
    checks.append("reader not running: plain explanation, what to do, host observer's stacks by name, no actions")
    shot(page, screens, "stacks-reader-down-1440.png", full_page=True)
    page.close()

    # A failed first read: friendly error with a retry, and the observer's stacks.
    page = browser.new_page(viewport={"width": 1440, "height": 900})
    watch(page, problems)
    fixture = Household(HOST)
    fixture.stacks_mode = "error"
    page.route(f"{BASE}/**", fixture.respond)
    page.goto(f"{BASE}/stacks")
    alert = page.get_by_role("alert").filter(has_text="Stacks can't be read right now")
    alert.wait_for()
    page.locator('article[aria-label="Observed stack downloads"]').wait_for()
    fixture.stacks_mode = "household"
    alert.get_by_role("button", name="Try again").click()
    page.locator('article[aria-label="Stack media"]').wait_for()
    checks.append("failed read: alert with retry, observer's stacks meanwhile, retry recovers")
    page.close()


def viewer_checks(browser, checks, problems):
    page = browser.new_page(viewport={"width": 1100, "height": 850})
    watch(page, problems)
    fixture = Household(HOST, role="viewer")
    page.route(f"{BASE}/**", fixture.respond)
    page.goto(f"{BASE}/stacks")
    media = page.locator('article[aria-label="Stack media"]')
    media.wait_for()
    settle(page)
    assert "jellyfin" in media.inner_text()
    assert page.get_by_role("group", name="Whole-stack actions for media").count() == 0
    assert page.get_by_role("button", name="Restart stack").count() == 0
    assert not any(path.startswith("/api/v1/stack/") for path, _ in fixture.calls), fixture.calls
    checks.append("viewer: cards and names, no lifecycle controls and no job reads")
    page.close()


def mobile_checks(browser, checks, problems, screens):
    page = browser.new_page(viewport={"width": 390, "height": 844}, has_touch=True)
    watch(page, problems)
    fixture = Household(HOST)
    page.route(f"{BASE}/**", fixture.respond)
    page.goto(f"{BASE}/stacks")
    page.locator('article[aria-label="Stack media"]').wait_for()
    settle(page)
    assert no_overflow(page)
    small = small_targets(page)
    assert not small, small
    shot(page, screens, "stacks-390.png", full_page=True)
    page.locator('article[aria-label="Stack media"] summary').click()
    assert no_overflow(page), "the member table scrolls inside its region"
    fixture.stacks_mode = "reader_down"
    page.get_by_role("button", name="Refresh stacks").click()
    page.get_by_text("The stack reader isn't running").wait_for()
    assert no_overflow(page)
    shot(page, screens, "stacks-reader-down-390.png", full_page=True)
    checks.append("390 px: no overflow (cards, open disclosure, reader down), 44 px targets")
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
    print(f"Stack card browser checks passed ({len(checks)}).")


if __name__ == "__main__":
    main()
