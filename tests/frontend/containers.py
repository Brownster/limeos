#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.11"
# dependencies = ["playwright==1.62.0"]
# ///
"""The Containers page against the closed household fixture; no live host.

Filters, search, grouping and sorting (and their URL), the actions menu by
keyboard, a restart and a stop queued through plan and approval with the row
following the job, logs (tail, auto-refresh, wrap, truncation), the detail
drawer with mounts, a stale Docker source, an operator's and a viewer's view,
and 390 px. With --screens DIR, also writes the review screenshots.
"""

import argparse
import json
import re
import sys
from pathlib import Path

from playwright.sync_api import sync_playwright

sys.path.insert(0, str(Path(__file__).resolve().parent))
from containers_fixture import ContainersHousehold  # noqa: E402

HOST = "limeos-containers.invalid"
BASE = f"https://{HOST}"
HEX64 = re.compile(r"\b[0-9a-f]{64}\b")


def watch(page, problems):
    page.on("pageerror", lambda error: problems.append(f"pageerror: {error}"))
    page.on(
        "console",
        lambda message: problems.append(f"console: {message.text}")
        if message.type == "error" and "Failed to load resource" not in message.text
        else None,
    )


def no_overflow(page):
    return page.evaluate("document.documentElement.scrollWidth <= window.innerWidth + 1")


def names(page):
    return page.locator("a.container-name").all_inner_texts()


def query(page):
    return page.evaluate("location.search")


def focused(locator):
    return locator.evaluate("element => document.activeElement === element")


def until(page, expression, timeout=5000):
    """Poll an expression with evaluate (wait_for_function needs eval, which the CSP refuses)."""
    for _ in range(timeout // 50):
        if page.evaluate(expression):
            return
        page.wait_for_timeout(50)
    raise AssertionError(f"never true: {expression}")


def shot(page, screens, name, full=True):
    if screens:
        page.wait_for_timeout(250)
        page.screenshot(path=str(screens / name), full_page=full)


def open_page(browser, problems, width=1440, height=900, role="administrator", path="/containers"):
    page = browser.new_page(viewport={"width": width, "height": height}, has_touch=width < 600)
    watch(page, problems)
    fixture = ContainersHousehold(HOST, role=role)
    page.route(f"{BASE}/**", fixture.respond)
    page.goto(f"{BASE}{path}")
    page.get_by_role("heading", name="Containers", level=1).wait_for()
    page.locator("a.container-name").first.wait_for()
    return page, fixture


def desktop(browser, checks, problems, screens):
    page, fixture = open_page(browser, problems)
    table = page.locator("table.containers-table")
    table.wait_for()
    assert len(names(page)) == 11, names(page)
    assert not HEX64.search(table.inner_text()), "a full container ID is shown in the list"
    assert table.locator("tbody tr").first.locator("a.container-name").inner_text() == "audiobookshelf"
    assert "lscr.io/" in table.inner_text() and "jellyfin/jellyfin:10.10.3" in table.inner_text()
    header = page.locator(".page-description").inner_text()
    assert "8 running" in header and "3 stopped" in header and "2 need attention" in header, header
    assert page.get_by_role("progressbar", name="CPU of prowlarr").get_attribute("aria-valuetext") == "91.0%, very high"
    assert page.get_by_role("progressbar", name="Memory of lidarr").get_attribute("aria-valuetext") == "Not running"
    assert page.get_by_role("button", name="Restart", exact=True).count() == 8
    assert page.get_by_role("button", name="Start", exact=True).count() == 3
    shot(page, screens, "list-1440.png")
    checks.append("1440: table of all 11 containers, readable images, no 64-character IDs, bars on overview thresholds")

    # Filters, search, sort and grouping, all reflected in the URL and kept on reload.
    show = page.get_by_role("group", name="Show")
    show.get_by_role("button", name="Needs attention").click()
    assert sorted(names(page)) == ["lidarr", "transmission"], names(page)
    assert query(page) == "?filter=attention"
    show.get_by_role("button", name="Running").click()
    assert len(names(page)) == 8 and "lidarr" not in names(page)
    show.get_by_role("button", name="Stopped").click()
    assert sorted(names(page)) == ["lidarr", "photo-import", "watchtower"]
    show.get_by_role("button", name="All").click()
    search = page.get_by_role("searchbox", name="Search containers by name, image or stack")
    search.fill("download")
    assert sorted(names(page)) == ["jackett", "prowlarr", "transmission"], names(page)
    assert "Showing 3 of 11 containers" in page.locator(".result-line").inner_text()
    search.fill("ghcr")
    assert names(page) == ["prowlarr"]
    search.fill("nothing-matches-this")
    page.get_by_text("No containers match").wait_for()
    page.get_by_role("button", name="Clear filters").first.click()
    assert len(names(page)) == 11 and query(page) == ""
    page.get_by_label("Sort").select_option("cpu")
    assert names(page)[:3] == ["prowlarr", "jellyfin", "transmission"], names(page)
    assert names(page)[-1] in ("lidarr", "photo-import", "watchtower")
    page.get_by_label("Sort").select_option("name")
    show.get_by_role("button", name="Running").click()
    search.fill("media")
    shot(page, screens, "filtered-1440.png")
    page.reload()
    page.locator("a.container-name").first.wait_for()
    assert page.get_by_role("group", name="Show").get_by_role("button", name="Running").get_attribute("aria-pressed") == "true"
    assert page.get_by_role("searchbox").input_value() == "media"
    assert sorted(names(page)) == ["jellyfin", "radarr", "sonarr"]
    page.get_by_role("button", name="Clear filters").first.click()
    group = page.get_by_role("button", name="Group by stack")
    group.click()
    assert group.get_attribute("aria-pressed") == "true" and query(page) == "?group=stack"
    headings = page.locator(".group-heading").all_inner_texts()
    assert [h.split("\n")[0].strip() for h in headings] == ["downloads", "media", "Not in a stack"], headings
    assert "3 of 4 running" in headings[1] and "3 of 3 running" in headings[0] and "2 of 4 running" in headings[2], headings
    assert page.locator('th[scope="colgroup"]').count() == 3
    shot(page, screens, "grouped-1440.png")
    show.get_by_role("button", name="Needs attention").click()
    assert "1 shown" in page.locator(".group-heading").first.inner_text()
    page.goto(f"{BASE}/containers?group=stack&filter=stopped&sort=memory")
    page.locator(".group-heading").first.wait_for()
    assert page.get_by_role("group", name="Show").get_by_role("button", name="Stopped").get_attribute("aria-pressed") == "true"
    assert page.get_by_label("Sort").input_value() == "memory"
    page.goto(f"{BASE}/containers")
    page.locator("a.container-name").first.wait_for()
    checks.append("filters (incl. needs attention), search, sort and group by stack with running-of-total; URL reproduces the view")

    # Actions menu, keyboard only.
    more = page.get_by_role("button", name="More actions for jellyfin")
    more.focus()
    page.keyboard.press("Enter")
    menu = page.get_by_role("menu", name="More actions for jellyfin")
    menu.wait_for()
    assert more.get_attribute("aria-expanded") == "true"
    start = menu.get_by_role("menuitem", name="Start", exact=True)
    assert focused(start) and not start.is_enabled()
    assert "It is already running" in start.inner_text()
    page.keyboard.press("ArrowDown")
    assert focused(menu.get_by_role("menuitem", name="Stop", exact=True))
    page.keyboard.press("End")
    assert focused(menu.get_by_role("menuitem", name="Details", exact=True))
    page.keyboard.press("ArrowDown")
    assert focused(start)
    page.keyboard.press("l")
    assert focused(menu.get_by_role("menuitem", name="Logs", exact=True))
    shot(page, screens, "menu-1440.png", full=False)
    page.keyboard.press("Enter")  # an aria-disabled item does nothing; Logs opens the log dialog
    page.get_by_role("dialog", name="Logs · jellyfin").wait_for()
    page.keyboard.press("Escape")
    page.get_by_role("dialog", name="Logs · jellyfin").wait_for(state="hidden")
    assert focused(more), "focus did not return to the menu button"
    page.keyboard.press("ArrowDown")
    menu.wait_for()
    start = menu.get_by_role("menuitem", name="Start", exact=True)
    start.press("Enter")
    assert menu.is_visible() and page.locator("dialog[open]").count() == 0
    page.keyboard.press("Escape")
    menu.wait_for(state="hidden")
    assert focused(more)
    more.click()
    menu.wait_for()
    page.mouse.click(5, 5)
    menu.wait_for(state="hidden")
    checks.append("actions menu: Enter/arrows/Home/End/type-ahead, unavailable items focusable with a reason, Escape returns focus, click outside closes")

    # Restart through the primary action: preview, approve, the row follows the job.
    row = page.locator("tr", has=page.locator("a.container-name", has_text="sonarr"))
    restart = row.get_by_role("button", name="Restart", exact=True)
    restart.click()
    dialog = page.get_by_role("dialog", name="Restart sonarr")
    dialog.get_by_role("button", name="Approve and restart").wait_for()
    assert "linuxserver/sonarr:latest" in dialog.inner_text()
    shot(page, screens, "approve-1440.png", full=False)
    for _ in range(6):
        page.keyboard.press("Tab")
        assert page.evaluate("!!document.activeElement.closest('dialog')")
    fixture.hold_jobs = True
    dialog.get_by_role("button", name="Approve and restart").click()
    dialog.wait_for(state="hidden")
    assert focused(restart)
    badge = row.locator(".row-job")
    badge.wait_for()
    assert badge.inner_text().startswith("Restart queued"), badge.inner_text()
    assert not row.get_by_role("button", name="Restart", exact=True).is_enabled()
    page.get_by_role("heading", name="Recent container operations").wait_for()
    assert "Restart · sonarr · 000000000000" in page.locator(".job-row").first.inner_text()
    badge.click()
    until(page, "document.activeElement.classList.contains('job-row')")
    overviews = fixture.overview_calls
    fixture.hold_jobs = False
    page.locator(".job-row").first.get_by_text("succeeded", exact=True).wait_for(timeout=10000)
    badge.wait_for(state="detached")
    assert fixture.overview_calls > overviews, "the overview was not re-read when the job settled"
    checks.append("restart: preview, approval, row shows the job in flight and links to it, re-reads the overview when it settles")

    # Stop from the menu: the row becomes exited without a manual refresh.
    row = page.locator("tr", has=page.locator("a.container-name", has_text="radarr"))
    more = row.get_by_role("button", name="More actions for radarr")
    more.click()
    page.get_by_role("menuitem", name="Stop", exact=True).click()
    stop = page.get_by_role("dialog", name="Stop radarr")
    stop.get_by_role("button", name="Approve and stop").click()
    stop.wait_for(state="hidden")
    assert focused(more)
    row.get_by_text("exited", exact=True).wait_for(timeout=10000)
    row.get_by_role("button", name="Start", exact=True).wait_for()
    assert fixture.jobs[-1]["plan"]["operation"] == "stop"
    checks.append("stop from the actions menu: focus returns to the menu button, the row turns exited by itself")

    # Logs: tail, truncation, auto-refresh only while open and visible, wrap and copy.
    calls = len(fixture.log_calls)
    logs_button = page.locator("tr", has=page.locator("a.container-name", has_text="jellyfin")).get_by_role("button", name="Logs", exact=True)
    logs_button.click()
    logs = page.get_by_role("dialog", name="Logs · jellyfin")
    logs.locator(".container-logs").wait_for()
    assert fixture.log_calls[calls:] == [100]
    logs.get_by_text("Showing the last 100 lines.").wait_for()
    logs.get_by_label("Lines to show").select_option("200")
    logs.get_by_text("Output reached the size limit.", exact=True).wait_for()
    logs.get_by_text("Showing the newest 141 of the last 200 lines.").wait_for()
    logs.get_by_label("Lines to show").select_option("50")
    logs.get_by_text("Showing the last 50 lines.").wait_for()
    assert fixture.log_calls[calls:] == [100, 200, 50]
    assert logs.get_by_text("Output reached the size limit.", exact=True).count() == 0
    wrap = logs.get_by_role("button", name="Wrap lines")
    assert wrap.get_attribute("aria-pressed") == "true"
    wrap.click()
    assert "nowrap" in logs.locator(".container-logs").get_attribute("class")
    auto = logs.get_by_role("button", name="Auto-refresh off")
    auto.click()
    logs.get_by_role("button", name="Auto-refresh on").wait_for()
    page.wait_for_timeout(5600)
    assert len(fixture.log_calls) - calls == 4, fixture.log_calls[calls:]
    shot(page, screens, "logs-1440.png", full=False)
    page.keyboard.press("Escape")
    logs.wait_for(state="hidden")
    assert focused(logs_button)
    after = len(fixture.log_calls)
    page.wait_for_timeout(6000)
    assert len(fixture.log_calls) == after, "logs were read after the dialog closed"
    logs_button.click()
    logs.locator(".container-logs").wait_for()
    assert logs.get_by_role("button", name="Auto-refresh off").count() == 1
    assert fixture.log_calls[-1] == 50
    page.keyboard.press("Escape")
    lidarr = page.locator("tr", has=page.locator("a.container-name", has_text="lidarr"))
    lidarr.get_by_role("button", name="Logs", exact=True).click()
    page.get_by_role("dialog", name="Logs · lidarr").get_by_text("Showing all 3 lines").wait_for()
    page.keyboard.press("Escape")
    checks.append("logs: tail 50/100/200 sent to the API, truncation notice, line note, wrap, auto-refresh every 5 s that stops on close")

    # Detail drawer: deep link, identity with copy, operations, mounts read once.
    storage = fixture.storage_calls
    name = page.get_by_role("link", name="jellyfin", exact=True)
    name.click()
    drawer = page.get_by_role("dialog", name="jellyfin")
    drawer.wait_for()
    assert query(page) == "?container=jellyfin"
    text = drawer.inner_text()
    assert "0" * 63 + "1" in text and "Docker Hub" in text and "10.10.3" in text, text
    drawer.get_by_text("/srv/config/jellyfin").wait_for()
    assert "read-only" in drawer.inner_text() and "2026" in drawer.locator(".detail-facts").inner_text()
    assert fixture.storage_calls == storage + 1
    drawer.get_by_role("button", name="Copy container ID").wait_for()
    assert drawer.get_by_role("button", name="Start", exact=True).count() == 0
    shot(page, screens, "detail-1440.png", full=False)
    page.keyboard.press("Escape")
    drawer.wait_for(state="hidden")
    until(page, "location.search === ''")
    until(page, "document.activeElement && document.activeElement.textContent === 'jellyfin'")
    page.goto(f"{BASE}/containers?container=sonarr")
    drawer = page.get_by_role("dialog", name="sonarr")
    drawer.wait_for()
    drawer.get_by_text("Your recent operations").wait_for()
    drawer.locator(".detail-jobs li").first.get_by_text("succeeded").wait_for()
    drawer.get_by_role("link", name="Details").first.click()
    drawer.wait_for(state="hidden")
    until(page, "document.activeElement.classList.contains('job-row')")
    page.goto(f"{BASE}/containers?container=no-such-thing")
    page.get_by_text("No container named “no-such-thing” is reported").wait_for()
    page.keyboard.press("Escape")
    checks.append("detail drawer: deep link, full ID with copy, image parts, start time and mounts (one read), operations link, focus return")

    # A stale Docker source: statuses kept with a warning, readings unknown.
    fixture.stale_docker = True
    page.get_by_role("button", name="Refresh now").click()
    page.get_by_text("Docker readings are out of date").wait_for()
    assert page.get_by_role("progressbar", name="CPU of prowlarr").get_attribute("aria-valuetext").startswith("Unknown")
    assert "91.0%" not in page.locator("table.containers-table").inner_text()
    shot(page, screens, "stale-1440.png", full=False)
    fixture.stale_docker = False
    fixture.overview_down = True
    page.get_by_role("button", name="Refresh now").click()
    page.get_by_role("alert").filter(has_text="The latest observation").wait_for()
    fixture.overview_down = False
    checks.append("stale Docker source: warning, CPU and memory Unknown; a failed refresh keeps the list and says so")
    page.close()


def first_load(browser, checks, problems):
    page = browser.new_page(viewport={"width": 1440, "height": 900})
    watch(page, problems)
    fixture = ContainersHousehold(HOST)
    fixture.overview_down = True
    page.route(f"{BASE}/**", fixture.respond)
    page.goto(f"{BASE}/containers")
    alert = page.get_by_role("alert").filter(has_text="Host observations")
    alert.wait_for()
    fixture.overview_down = False
    alert.get_by_role("button", name="Try again").click()
    page.locator("a.container-name").first.wait_for()
    fixture.containers.clear()
    page.get_by_role("button", name="Refresh now").click()
    page.get_by_text("No containers reported").wait_for()
    checks.append("first load failing shows an error with retry; an empty host says no containers are reported")
    page.close()


def small_targets(page):
    """Everything pressable in the page below 44 px high (text inside a paragraph excepted)."""
    return page.evaluate(
        """() => [...document.querySelectorAll('main button, main a, main input, main select')]
          .filter(e => e.getClientRects().length && !e.closest('.state-support, p'))
          .map(e => [e.textContent.trim() || e.getAttribute('aria-label'), Math.round(e.getBoundingClientRect().height)])
          .filter(([, h]) => h < 44)"""
    )


def mobile(browser, checks, problems, screens):
    page, fixture = open_page(browser, problems, width=390, height=844)
    assert page.locator("table.containers-table").count() == 0
    assert page.locator("li.container-card").count() == 11
    assert no_overflow(page)
    assert not small_targets(page), small_targets(page)
    shot(page, screens, "list-390.png")
    page.get_by_role("button", name="Group by stack").click()
    assert no_overflow(page)
    assert not small_targets(page), small_targets(page)
    shot(page, screens, "grouped-390.png")
    more = page.get_by_role("button", name="More actions for transmission")
    more.scroll_into_view_if_needed()
    more.click()
    menu = page.get_by_role("menu", name="More actions for transmission")
    menu.wait_for()
    box = menu.bounding_box()
    assert box["x"] >= 0 and box["x"] + box["width"] <= 390, box
    shot(page, screens, "menu-390.png", full=False)
    page.keyboard.press("Escape")
    page.get_by_role("link", name="jellyfin", exact=True).click()
    drawer = page.get_by_role("dialog", name="jellyfin")
    drawer.wait_for()
    box = drawer.bounding_box()
    assert box["width"] <= 390.5, box
    assert no_overflow(page)
    shot(page, screens, "detail-390.png", full=False)
    page.keyboard.press("Escape")
    page.locator("li.container-card", has=page.locator("a.container-name", has_text="jellyfin")).get_by_role("button", name="Logs", exact=True).click()
    page.get_by_role("dialog", name="Logs · jellyfin").locator(".container-logs").wait_for()
    assert no_overflow(page)
    shot(page, screens, "logs-390.png", full=False)
    page.keyboard.press("Escape")
    checks.append("390 px: cards, no horizontal overflow, 44 px targets, menu inside the viewport, full-width drawer and logs")
    page.close()


def operator(browser, checks, problems):
    page, fixture = open_page(browser, problems, role="operator", path="/containers?container=jellyfin")
    drawer = page.get_by_role("dialog", name="jellyfin")
    drawer.wait_for()
    drawer.get_by_text("Storage mounts are shown to accounts that may manage storage").wait_for()
    drawer.get_by_role("button", name="Restart", exact=True).wait_for()
    assert fixture.storage_calls == 0
    checks.append("operator: lifecycle controls, no storage read and a plain explanation instead of mounts")
    page.close()


def viewer(browser, checks, problems, screens):
    page, fixture = open_page(browser, problems, role="viewer")
    for label in ["Start", "Stop", "Restart", "Logs"]:
        assert page.get_by_role("button", name=label, exact=True).count() == 0, label
    assert page.get_by_role("button", name=re.compile("^More actions")).count() == 0
    assert page.get_by_role("heading", name="Recent container operations").count() == 0
    page.get_by_role("link", name="sonarr", exact=True).click()
    drawer = page.get_by_role("dialog", name="sonarr")
    drawer.wait_for()
    assert drawer.get_by_role("button", name="Restart", exact=True).count() == 0
    assert "Your recent operations" not in drawer.inner_text()
    page.keyboard.press("Escape")
    shot(page, screens, "viewer-1440.png")
    forbidden = [p for p, _ in fixture.calls if p.startswith(("/api/v1/container", "/api/v1/storage", "/api/v1/jobs"))]
    assert not forbidden, forbidden
    checks.append("viewer: no lifecycle, log or menu controls, no operations list and no reads for them")
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
            desktop(browser, checks, problems, args.screens)
            first_load(browser, checks, problems)
            mobile(browser, checks, problems, args.screens)
            operator(browser, checks, problems)
            viewer(browser, checks, problems, args.screens)
            assert not problems, problems
        finally:
            browser.close()
    (args.output / "result.json").write_text(
        json.dumps({"checks": checks, "live_host_operations": 0}, indent=2) + "\n"
    )
    print(f"Containers browser checks passed ({len(checks)}).")


if __name__ == "__main__":
    main()
