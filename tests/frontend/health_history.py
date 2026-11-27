#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.11"
# dependencies = ["playwright==1.62.0"]
# ///
"""Mounted history/acknowledgment UI against a closed fixture, never Core/Root."""

import argparse
import hashlib
import json
import mimetypes
import sys
import time
from pathlib import Path
from urllib.parse import parse_qs, urlsplit

from playwright.sync_api import Error, sync_playwright

sys.path.insert(0, str(Path(__file__).resolve().parent))
from browser_checks import fits_viewport, forbid_page_eval, scrolls_horizontally  # noqa: E402

ROOT = Path(__file__).resolve().parents[2]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    build = json.loads(
        (ROOT / ".cache/b1-history-qualification/frontend-build.json").read_text()
    )
    assert build["exit_code"] == 0 and build["inputs_unchanged"]
    for name, expected in build["inputs"].items():
        assert hashlib.sha256((ROOT / name).read_bytes()).hexdigest() == expected
    for name, expected in build["ui"].items():
        assert hashlib.sha256((ROOT / name).read_bytes()).hexdigest() == expected
    served = {}
    session = {
        "principal": {"id": "fixture-viewer", "role": "viewer", "grant_revision": 1},
        "csrf_token": "d" * 64,
        "expires_at": int(time.time()) + 3600,
    }
    alert = {
        "key": "mount:/Case/Data",
        "kind": "mount",
        "last_sequence": 2,
        "incident": {
            "phase": "open",
            "generation": 2,
            "severity": "warning",
            "summary": "Fixture mount incident <script>must stay text</script>",
            "announced": True,
        },
    }
    notes = []
    for identifier, state in (("a", "delivered"), ("b", "pending"), ("c", "delivered")):
        notes.append(
            {
                "id": identifier * 64,
                "revision": 1 if state == "pending" else 3,
                "event": {
                    "key": "mount:/Case/Data",
                    "generation": 2,
                    "event": "incident",
                    "kind": "mount",
                    "severity": "warning",
                    "summary": f"Fixture notification {identifier}",
                    "at": 100,
                },
                "channel": "in_app",
                "delivery": {
                    "state": state,
                    **({"at": 101} if state == "delivered" else {}),
                },
                "attempts": 0 if state == "pending" else 1,
                "acknowledged_at": None,
            }
        )
    calls, errors, held = [], [], []
    unavailable = forbidden = hold = False
    late = None
    with sync_playwright() as playwright:
        browser = playwright.chromium.launch(
            executable_path="/usr/bin/google-chrome",
            headless=True,
            args=["--disable-dev-shm-usage"],
        )
        page = browser.new_page(viewport={"width": 1100, "height": 900})
        forbid_page_eval(page)
        page.on("pageerror", lambda error: errors.append(str(error)))

        def respond(route):
            nonlocal hold
            request = route.request
            url = urlsplit(request.url)
            assert url.scheme == "https" and url.netloc == "limeos-history.invalid"
            path, query = url.path, parse_qs(url.query)
            if path.startswith("/api/"):
                body = request.post_data_json if request.post_data else None
                calls.append(
                    {
                        "path": path,
                        "method": request.method,
                        "keys": sorted(body) if isinstance(body, dict) else [],
                        "after": query.get("after"),
                    }
                )
                status = 200
                if path == "/api/v1/auth/session":
                    data = session
                elif (
                    path
                    in ("/api/v1/automation/alerts", "/api/v1/automation/notifications")
                    and request.method == "GET"
                ):
                    if hold and path.endswith("/notifications"):
                        hold = False
                        held.append(route)
                        return
                    if forbidden:
                        status, data = 403, {"private": "MUST_NOT_RENDER"}
                    elif unavailable:
                        status, data = (
                            503,
                            {
                                "code": "unavailable",
                                "message": "Fixture history unavailable",
                                "retry": True,
                                "audit_id": "fixture-history",
                            },
                        )
                    elif path.endswith("/alerts"):
                        rows = [alert] if not query else []
                        data = {
                            "items": rows,
                            "next": alert["key"] if not query else None,
                            "partial": True,
                        }
                    else:
                        data = {
                            "items": notes if not query else [],
                            "next": None,
                            "partial": True,
                        }
                elif path.endswith("/acknowledgment") and request.method == "POST":
                    assert request.headers["x-csrf-token"] == session["csrf_token"]
                    assert set(body) == {"revision"}
                    identifier = path.split("/")[-2]
                    row = next(row for row in notes if row["id"] == identifier)
                    assert (
                        row["delivery"]["state"] == "delivered"
                        and row["revision"] == body["revision"]
                    )
                    row.update(revision=row["revision"] + 1, acknowledged_at=102)
                    if identifier == "c" * 64:
                        route.abort(
                            "failed"
                        )  # Model a committed acknowledgment with a lost reply.
                        return
                    data = row
                elif (
                    path
                    in (
                        "/api/v1/overview",
                        "/api/v1/system/history",
                        "/api/v1/observations/stream",
                    )
                    and request.method == "GET"
                ):
                    status, data = (
                        503,
                        {
                            "code": "unavailable",
                            "message": "Fixture overview unavailable",
                            "retry": True,
                            "audit_id": "fixture-overview",
                        },
                    )
                else:
                    raise AssertionError(
                        f"Unexpected fixture request: {request.method} {path}"
                    )
                route.fulfill(
                    status=status,
                    content_type="application/json",
                    body=json.dumps(data),
                )
                return
            if path == "/favicon.ico":
                route.fulfill(status=204)
                return
            asset = ROOT / "frontend/dist" / path.lstrip("/")
            assert asset.resolve().is_relative_to((ROOT / "frontend/dist").resolve())
            if not asset.is_file():
                # Caddy: try_files {path} /index.html (history-API routes).
                asset = ROOT / "frontend/dist/index.html"
            content = asset.read_bytes()
            name = str(asset.relative_to(ROOT))
            assert hashlib.sha256(content).hexdigest() == build["ui"][name]
            served[name] = build["ui"][name]
            route.fulfill(
                status=200,
                content_type=mimetypes.guess_type(str(asset))[0]
                or "application/octet-stream",
                body=content,
            )

        page.route("**/*", respond)
        try:
            page.goto("https://limeos-history.invalid/")
            # Alert history is the Alerts page of the app shell (was a toggle button).
            button = page.get_by_role("link", name="Alerts", exact=True)
            button.wait_for()
            assert not any("/automation/" in call["path"] for call in calls)
            button.click()
            page.get_by_text(alert["incident"]["summary"], exact=True).wait_for()
            assert page.locator("script").filter(has_text="must stay text").count() == 0
            pending = page.get_by_role("row").filter(has_text="Fixture notification b")
            assert pending.get_by_role("button").count() == 0
            row = page.get_by_role("row").filter(has_text="Fixture notification a")
            row.get_by_role(
                "button", name="Acknowledge incident for mount:/Case/Data", exact=True
            ).click()
            row.get_by_text("Acknowledged", exact=True).wait_for()
            row = page.get_by_role("row").filter(has_text="Fixture notification c")
            row.get_by_role(
                "button", name="Acknowledge incident for mount:/Case/Data", exact=True
            ).click()
            row.get_by_text("Acknowledged", exact=True).wait_for()
            assert len([call for call in calls if call["method"] == "POST"]) == 2
            page.get_by_role("button", name="Next alerts", exact=True).click()
            page.get_by_text("No alert history on this page.", exact=True).wait_for()
            assert any(call["after"] == ["mount:/Case/Data"] for call in calls)
            page.get_by_role("button", name="First alerts", exact=True).click()
            page.get_by_text(alert["incident"]["summary"], exact=True).wait_for()
            hold = True
            page.get_by_role("button", name="Refresh alert history", exact=True).click()
            deadline = time.monotonic() + 5
            while not held:
                assert time.monotonic() < deadline, (
                    "pending history read did not arrive"
                )
                page.wait_for_timeout(10)
            assert len(held) == 1
            page.get_by_role("link", name="Overview", exact=True).click()
            page.get_by_role("heading", name="Overview", level=1).wait_for()
            page.get_by_role("link", name="Alerts", exact=True).click()
            page.get_by_text(alert["incident"]["summary"], exact=True).wait_for()
            try:
                held[0].fulfill(status=401, content_type="application/json", body="{}")
                late = "late 401 delivered after owner disposal"
            except Error:
                late = "old transport canceled after owner disposal"
            assert page.locator(".account").is_visible()
            page.screenshot(
                path=str(args.output / "history-desktop.png"), full_page=True
            )
            page.set_viewport_size({"width": 390, "height": 844})
            assert fits_viewport(page)
            page.screenshot(
                path=str(args.output / "history-mobile.png"), full_page=True
            )
            region = page.get_by_role(
                "region", name="Recorded notification delivery", exact=True
            )
            assert scrolls_horizontally(page, region, region.locator("th").first)
            page.screenshot(
                path=str(args.output / "history-mobile-delivery.png"), full_page=True
            )
            unavailable = True
            page.get_by_role("button", name="Refresh alert history", exact=True).click()
            page.get_by_text(
                "Fixture history unavailable", exact=False
            ).first.wait_for()
            assert page.get_by_text(
                "Previously read notifications may be out of date.", exact=False
            ).is_visible()
            forbidden = True
            page.get_by_role("button", name="Refresh alert history", exact=True).click()
            page.get_by_text(
                "The request is outside the permitted scope.", exact=False
            ).first.wait_for()
            assert alert["incident"]["summary"] not in page.locator("body").inner_text()
            assert "MUST_NOT_RENDER" not in page.content()
            assert len([call for call in calls if call["method"] == "POST"]) == 2
            assert not any("/schedules" in call["path"] for call in calls)
            assert not errors, errors
            (args.output / "result.json").write_text(
                json.dumps(
                    {
                        "scope": "closed browser fixture only; no live Core/Store, protected collector, host effect or cutover acceptance",
                        "browser": browser.version,
                        "late_disposed_response": late,
                        "ui": build["ui"],
                        "served_ui": served,
                        "checks": [
                            "mounted history with unavailable overview",
                            "literal keys and escaped summaries",
                            "delivered-only acknowledgment",
                            "lost reply reconciled by GET without repeat POST",
                            "literal cursor paging",
                            "closed owner cannot expire replacement",
                            "stale and forbidden state",
                            "desktop/mobile layout",
                            "no schedule access or send endpoint",
                        ],
                        "calls": calls,
                        "page_errors": errors,
                    },
                    indent=2,
                )
                + "\n"
            )
        except BaseException:
            page.screenshot(path=str(args.output / "failure.png"), full_page=True)
            (args.output / "failure.txt").write_text(
                page.locator("body").inner_text()[:16000]
            )
            raise
        finally:
            browser.close()


if __name__ == "__main__":
    main()
