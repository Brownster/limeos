#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.11"
# dependencies = ["playwright==1.62.0"]
# ///
"""App shell, routes, drawer and Overview against the closed household fixture.

Serves the built UI as Caddy does (try_files, production CSP) and answers
every API request from tests/frontend/household_fixture.py; no live host.
With --screens DIR, also writes the PNGs for each page at 1440x900 and
390x844.
"""

import argparse
import json
import sys
from pathlib import Path

from playwright.sync_api import sync_playwright

sys.path.insert(0, str(Path(__file__).resolve().parent))
from household_fixture import Household  # noqa: E402

HOST = "limeos-shell.invalid"
BASE = f"https://{HOST}"
PAGES = [
    ("overview", "/", "Overview", "Overview"),
    ("containers", "/containers", "Containers", "Containers"),
    ("stacks", "/stacks", "Stacks", "Stacks"),
    ("storage", "/storage", "Storage", "Storage"),
    ("alerts", "/alerts", "Alerts", "Alerts"),
]


def no_overflow(page):
    return page.evaluate(
        "document.documentElement.scrollWidth <= window.innerWidth + 1"
    )


def h1(page):
    return page.locator("h1").first.inner_text()


def watch(page, problems):
    page.on("pageerror", lambda error: problems.append(f"pageerror: {error}"))
    page.on(
        "console",
        lambda message: problems.append(f"console: {message.text}")
        if message.type == "error"
        and "Failed to load resource" not in message.text
        else None,
    )


def settle(page):
    page.wait_for_load_state("networkidle")
    page.wait_for_timeout(250)


def desktop_checks(browser, checks, problems, screens):
    page = browser.new_page(viewport={"width": 1440, "height": 900})
    watch(page, problems)
    fixture = Household(HOST, signed_in=False)
    page.route(f"{BASE}/**", fixture.respond)

    # A deep link while signed out shows the sign-in page, then the page itself.
    page.goto(f"{BASE}/stacks")
    page.get_by_role("heading", name="Welcome home.").wait_for()
    assert page.title() == "LimeOS · Sign in"
    if screens:
        page.screenshot(path=str(screens / "login-1440.png"), full_page=True)
    page.get_by_label("Username").fill("alex")
    page.get_by_label("Password").fill("synthetic-password")
    page.get_by_role("button", name="Sign in").click()
    page.get_by_role("heading", name="Stacks", level=1).wait_for()
    assert page.url == f"{BASE}/stacks"
    checks.append("signed-out deep link signs in to the same page")

    # Sidebar navigation: URL, heading, title, aria-current and focus.
    for name, path, heading, title in PAGES:
        page.get_by_role("navigation", name="Primary").get_by_role(
            "link", name=heading, exact=True
        ).click()
        page.wait_for_url(f"{BASE}{path}")
        page.get_by_role("heading", name=heading, level=1).wait_for()
        assert page.title() == f"LimeOS · {title}", page.title()
        current = page.locator('.sidebar [aria-current="page"]')
        assert current.count() == 1 and current.inner_text() == heading
        page.wait_for_function(
            "document.activeElement && document.activeElement.matches('h1')"
        )
        settle(page)
        assert no_overflow(page), name
        if screens:
            page.screenshot(path=str(screens / f"{name}-1440.png"), full_page=True)
    checks.append("sidebar links route, title, mark current and focus the heading")

    # Reload and deep links on every route, sub-routes and aliases.
    for path, heading in [
        ("/containers", "Containers"),
        ("/storage", "Storage"),
        ("/alerts/schedules", "Alerts"),
        ("/alerts/policy", "Alerts"),
    ]:
        page.goto(f"{BASE}{path}")
        page.get_by_role("heading", name=heading, level=1).wait_for()
        page.reload()
        page.get_by_role("heading", name=heading, level=1).wait_for()
    sub = page.get_by_role("navigation", name="Alerts sections")
    assert sub.locator('[aria-current="page"]').inner_text() == "Alert policy"
    page.get_by_role("button", name="Save health settings").wait_for()
    if screens:
        page.screenshot(path=str(screens / "alerts-policy-1440.png"), full_page=True)
    for alias, target in [("/disks", "/storage"), ("/system", "/"), ("/Containers/", "/containers")]:
        page.goto(f"{BASE}{alias}")
        page.wait_for_url(f"{BASE}{target}")
    page.goto(f"{BASE}/no-such-page")
    page.get_by_role("heading", name="Page not found", level=1).wait_for()
    assert page.title() == "LimeOS · Page not found"
    page.get_by_role("link", name="Overview").last.click()
    page.wait_for_url(f"{BASE}/")
    page.go_back()
    page.get_by_role("heading", name="Page not found", level=1).wait_for()
    checks.append("reload, deep links, aliases, back button and unknown paths")

    # Overview: banner, metric cards, facts and counts from the fixture.
    page.goto(f"{BASE}/")
    banner = page.locator(".health-banner")
    banner.get_by_role("heading", name="Something needs a look").wait_for()
    text = banner.inner_text()
    for fragment in ["transmission is unhealthy", "media stack is only partly running", "Root disk is high: 78.9%"]:
        assert fragment in text, text
    cards = page.locator(".metric-grid").inner_text()
    for fragment in ["23.4%", "61.2%", "78.9%", "58.3 °C", "1.42", "3d 4h"]:
        assert fragment in cards, cards
    disk = page.get_by_role("progressbar", name="Root disk")
    assert disk.get_attribute("aria-valuenow") == "79"
    workloads = page.locator('section[aria-labelledby="workloads-title"]').inner_text()
    for fragment in ["7 running", "1 unhealthy", "1 stopped", "1 partly running"]:
        assert fragment in workloads, workloads
    assert page.get_by_text("Live", exact=True).count() == 0  # the fixture has no stream
    page.get_by_text("Updated ", exact=False).first.wait_for()
    checks.append("overview banner, thresholds, load, uptime and workload counts")

    # Stale sources never read as healthy.
    fixture.stale_docker = True
    page.get_by_role("button", name="Refresh now").click()
    banner.get_by_text("docker readings are out of date").wait_for()
    assert "All systems normal" not in banner.inner_text()
    assert "out of date" in page.locator('section[aria-labelledby="workloads-title"]').inner_text()
    fixture.stale_docker = False
    checks.append("a stale source withholds 'healthy' and marks counts out of date")

    # Storage: the helper being unavailable is explained, not an error.
    page.get_by_role("link", name="Storage", exact=True).click()
    page.get_by_text("Disk mounting isn't available on this host right now.", exact=False).wait_for()
    assert page.locator('section[aria-labelledby="mount-heading"] [role="alert"]').count() == 0
    checks.append("storage helper unavailable is explained without an alert")

    # The overview failing is said in friendly words, reference kept for support.
    fixture.overview_down = True
    page.get_by_role("button", name="Refresh now").click()
    alert = page.get_by_role("alert").filter(has_text="The latest observation")
    alert.wait_for()
    assert "fixture-overview" in alert.inner_text() or alert.locator("details").count() == 1
    fixture.overview_down = False
    checks.append("failed refresh keeps the last observation and says so")
    page.close()
    return fixture


def mobile_checks(browser, checks, problems, screens):
    page = browser.new_page(viewport={"width": 390, "height": 844}, has_touch=True)
    watch(page, problems)
    fixture = Household(HOST, signed_in=False)
    page.route(f"{BASE}/**", fixture.respond)
    page.goto(f"{BASE}/")
    page.get_by_role("heading", name="Welcome home.").wait_for()
    assert no_overflow(page)
    if screens:
        page.screenshot(path=str(screens / "login-390.png"), full_page=True)
    fixture.signed_in = True
    page.reload()
    page.get_by_role("heading", name="Overview", level=1).wait_for()
    assert not page.locator(".sidebar").is_visible()
    menu = page.get_by_role("button", name="Open navigation")
    for name, path, heading, _ in PAGES:
        menu.click()
        drawer = page.get_by_role("dialog", name="Navigation")
        drawer.wait_for(state="visible")
        assert page.evaluate("!!document.activeElement.closest('dialog')")
        if name == "overview":
            for _ in range(10):
                page.keyboard.press("Tab")
                assert page.evaluate("!!document.activeElement.closest('dialog')")
            for _ in range(3):
                page.keyboard.press("Shift+Tab")
                assert page.evaluate("!!document.activeElement.closest('dialog')")
            sizes = drawer.evaluate(
                "d => [...d.querySelectorAll('a, button')].map(e => e.getBoundingClientRect().height)"
            )
            assert min(sizes) >= 44, sizes
            if screens:
                page.screenshot(path=str(screens / "drawer-390.png"))
            page.keyboard.press("Escape")
            drawer.wait_for(state="hidden")
            assert menu.evaluate("e => document.activeElement === e")
            menu.click()
            drawer.wait_for(state="visible")
        drawer.get_by_role("link", name=heading, exact=True).click()
        drawer.wait_for(state="hidden")
        page.wait_for_url(f"{BASE}{path}")
        page.get_by_role("heading", name=heading, level=1).wait_for()
        settle(page)
        assert no_overflow(page), name
        small = page.evaluate(
            """() => [...document.querySelectorAll('main button, main a, header button')]
              .filter(e => e.getClientRects().length && !e.closest('table, .state-support, .link-list, p'))
              .map(e => [e.textContent.trim() || e.getAttribute('aria-label'), Math.round(e.getBoundingClientRect().height)])
              .filter(([, h]) => h < 44)"""
        )
        assert not small, (name, small)
        if screens:
            page.screenshot(path=str(screens / f"{name}-390.png"), full_page=True)
    checks.append("390 px: drawer contains focus, Escape returns it, links route, no overflow, 44 px targets")
    page.close()


def viewer_checks(browser, checks, problems):
    page = browser.new_page(viewport={"width": 1100, "height": 850})
    watch(page, problems)
    fixture = Household(HOST, role="viewer")
    page.route(f"{BASE}/**", fixture.respond)
    page.goto(f"{BASE}/alerts")
    page.get_by_role("heading", name="Recorded alerts").wait_for()
    assert page.get_by_role("navigation", name="Alerts sections").count() == 0
    page.goto(f"{BASE}/alerts/policy")
    page.get_by_text("Only operators and administrators can change health checks").wait_for()
    assert page.get_by_role("button", name="Save health settings").count() == 0
    page.goto(f"{BASE}/storage")
    page.get_by_role("heading", name="Storage", level=1).wait_for()
    settle(page)
    assert page.locator("#mount-heading").count() == 0
    page.goto(f"{BASE}/stacks")
    page.locator('article[aria-label="Stack media"]').wait_for()
    assert page.get_by_role("group", name="Whole-stack actions for media").count() == 0
    page.goto(f"{BASE}/")
    page.locator(".health-banner").wait_for()
    assert page.get_by_role("link", name="Manage alert policy").count() == 0
    assert not any(
        path.startswith("/api/v1/automation/health-policy") or path.startswith("/api/v1/storage/")
        for path, _ in fixture.calls
    ), fixture.calls
    checks.append("viewer: no schedule/policy editors, mounts or stack actions, and no reads for them")
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
            desktop_checks(browser, checks, problems, args.screens)
            mobile_checks(browser, checks, problems, args.screens)
            viewer_checks(browser, checks, problems)
            assert not problems, problems
        finally:
            browser.close()
    (args.output / "result.json").write_text(
        json.dumps({"checks": checks, "live_host_operations": 0}, indent=2) + "\n"
    )
    print(f"Shell browser checks passed ({len(checks)}).")


if __name__ == "__main__":
    main()
