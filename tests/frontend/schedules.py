#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.11"
# dependencies = ["playwright==1.62.0"]
# ///
"""Mounted schedule flow against a closed API fixture; no live host effects."""

import argparse
import json
import mimetypes
import sys
import time
from pathlib import Path

from playwright.sync_api import Error, sync_playwright

sys.path.insert(0, str(Path(__file__).resolve().parent))
from browser_checks import fits_viewport, forbid_page_eval  # noqa: E402

ROOT = Path(__file__).resolve().parents[2]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    identifier = "a" * 64
    session = {
        "principal": {
            "id": "fixture-admin",
            "role": "administrator",
            "grant_revision": 1,
        },
        "csrf_token": "b" * 64,
        "expires_at": int(time.time()) + 3600,
    }
    rows = []
    calls = []
    errors = []
    enabled = False
    unavailable = False
    forbidden = False
    hold_enablement = False
    held = []
    late_delivery = None
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
            nonlocal enabled, session
            request = route.request
            assert request.url.startswith("https://limeos-schedules.invalid/")
            path = request.url.split("limeos-schedules.invalid", 1)[1].split("?", 1)[0]
            if path.startswith("/api/"):
                body = request.post_data_json if request.post_data else None
                # Credentials never enter the fixture result/log.
                calls.append(
                    {
                        "path": path,
                        "method": request.method,
                        "keys": sorted(body) if isinstance(body, dict) else [],
                    }
                )
                status = 200
                if path == "/api/v1/auth/session" and request.method == "GET":
                    data = session
                elif (
                    path == "/api/v1/auth/capabilities/observe-health/enable"
                    and request.method == "POST"
                ):
                    assert (
                        set(body) == {"password"}
                        and body["password"] == "synthetic-password"
                    )
                    assert request.headers["x-csrf-token"] == session["csrf_token"]
                    if hold_enablement:
                        held.append(route)
                        return
                    needs_sign_in = not enabled
                    enabled = True
                    data = {"enabled": True, "reauthenticate": needs_sign_in}
                elif path == "/api/v1/auth/login" and request.method == "POST":
                    assert body == {
                        "username": "fixture-admin",
                        "password": "synthetic-password",
                    }
                    session = {
                        **session,
                        "principal": {**session["principal"], "grant_revision": 2},
                        "csrf_token": "c" * 64,
                    }
                    data = session
                elif path == "/api/v1/automation/schedules" and request.method == "GET":
                    if forbidden:
                        status, data = 403, {"private": "MUST_NOT_RENDER"}
                    elif unavailable:
                        status, data = (
                            503,
                            {
                                "code": "unavailable",
                                "message": "Schedule source unavailable",
                                "retry": True,
                                "audit_id": "fixture-source",
                            },
                        )
                    else:
                        data = {"items": rows, "next": None, "partial": False}
                elif (
                    path == "/api/v1/automation/schedules" and request.method == "POST"
                ):
                    assert request.headers["x-csrf-token"] == session["csrf_token"]
                    assert set(body) == {
                        "interval_seconds",
                        "grace_seconds",
                        "missed",
                        "window",
                    }
                    if not enabled:
                        status, data = (
                            403,
                            {
                                "code": "forbidden",
                                "message": "Health scheduling permission required",
                                "retry": False,
                                "audit_id": "fixture-permission",
                            },
                        )
                    else:
                        assert body["interval_seconds"] == 120 and body["window"] == {
                            "start_minute": 1410,
                            "end_minute": 60,
                        }
                        rows.append(
                            {
                                "schedule": {
                                    "id": identifier,
                                    "revision": 1,
                                    "paused": False,
                                    "settled_through": None,
                                    "definition": {
                                        "operation": "observe_health",
                                        "anchor": int(time.time()),
                                        **body,
                                    },
                                },
                                "status": "awaiting_producer",
                                "last_occurrence": None,
                            }
                        )
                        status, data = 201, rows[0]
                elif (
                    path == f"/api/v1/automation/schedules/{identifier}/pause"
                    and request.method == "POST"
                ):
                    assert request.headers["x-csrf-token"] == session["csrf_token"]
                    assert (
                        set(body) == {"revision", "paused"}
                        and body["revision"] == rows[0]["schedule"]["revision"]
                    )
                    rows[0]["schedule"].update(
                        revision=body["revision"] + 1, paused=body["paused"]
                    )
                    rows[0]["status"] = (
                        "paused" if body["paused"] else "awaiting_producer"
                    )
                    data = rows[0]
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
                            "message": "Observations unavailable",
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
            route.fulfill(
                status=200,
                content_type=mimetypes.guess_type(str(asset))[0]
                or "application/octet-stream",
                body=asset.read_bytes(),
            )

        page.route("**/*", respond)
        try:
            # Schedules are a sub-page of Alerts, linked from the Overview (was a toggle button).
            page.goto("https://limeos-schedules.invalid/")
            page.get_by_role("link", name="Health schedules", exact=True).click()
            page.get_by_text("No schedules on this page.", exact=True).wait_for()
            page.get_by_role(
                "button", name="Create observation schedule", exact=True
            ).click()
            page.get_by_text(
                "Health scheduling permission required", exact=False
            ).wait_for()
            assert page.get_by_role(
                "button", name="Create observation schedule", exact=True
            ).is_disabled()
            page.get_by_label("Current password", exact=True).fill("synthetic-password")
            page.get_by_role(
                "button", name="Enable health scheduling", exact=True
            ).click()
            page.get_by_text(
                "Health scheduling enabled. Sign in again to use the updated permission.",
                exact=True,
            ).wait_for()
            assert page.get_by_label("Password", exact=True).input_value() == ""
            page.get_by_label("Username", exact=True).fill("fixture-admin")
            page.get_by_label("Password", exact=True).fill("synthetic-password")
            page.get_by_role("button", name="Sign in", exact=True).click()
            # Signing in again returns to the same route.
            page.get_by_text("No schedules on this page.", exact=True).wait_for()
            page.get_by_label("Interval in minutes", exact=True).fill("2")
            page.get_by_label("Use a daily UTC maintenance window", exact=True).check()
            page.get_by_label("Window start (UTC)", exact=True).fill("23:30")
            page.get_by_label("Window end (UTC)", exact=True).fill("01:00")
            page.get_by_role(
                "button", name="Create observation schedule", exact=True
            ).click()
            page.get_by_role(
                "button", name=f"Pause {identifier}", exact=True
            ).wait_for()
            page.get_by_text("Waiting for observations", exact=True).wait_for()
            page.get_by_role("button", name=f"Pause {identifier}", exact=True).click()
            page.get_by_role(
                "button", name=f"Resume {identifier}", exact=True
            ).wait_for()
            page.get_by_role("button", name=f"Resume {identifier}", exact=True).click()
            page.get_by_role(
                "button", name=f"Pause {identifier}", exact=True
            ).wait_for()
            page.get_by_label("Current password", exact=True).fill("synthetic-password")
            page.get_by_role(
                "button", name="Enable health scheduling", exact=True
            ).click()
            page.get_by_text("Scheduling permission confirmed.", exact=True).wait_for()
            assert page.locator(".account").is_visible()
            page.get_by_role("link", name="Overview", exact=True).click()
            page.get_by_role("link", name="Health schedules", exact=True).click()
            page.get_by_role(
                "button", name=f"Pause {identifier}", exact=True
            ).wait_for()
            hold_enablement = True
            page.get_by_label("Current password", exact=True).fill("synthetic-password")
            page.get_by_role(
                "button", name="Enable health scheduling", exact=True
            ).click()
            page.get_by_role(
                "button", name="Confirming permission…", exact=True
            ).wait_for()
            assert len(held) == 1
            assert page.get_by_label("Current password", exact=True).input_value() == ""
            page.get_by_role("link", name="Overview", exact=True).click()
            assert page.get_by_label("Current password", exact=True).count() == 0
            page.get_by_role("link", name="Health schedules", exact=True).click()
            try:
                held[0].fulfill(status=401, content_type="application/json", body="{}")
                late_delivery = "late 401 fulfilled after owner disposal"
            except Error:
                late_delivery = "transport already canceled after owner disposal"
            page.get_by_role(
                "button", name=f"Pause {identifier}", exact=True
            ).wait_for()
            assert page.locator(".account").is_visible()
            page.screenshot(path=str(args.output / "schedules.png"), full_page=True)
            page.set_viewport_size({"width": 390, "height": 844})
            assert page.locator(".schedules-table").bounding_box()["height"] < 500
            assert fits_viewport(page)
            page.screenshot(
                path=str(args.output / "schedules-mobile.png"), full_page=True
            )
            unavailable = True
            page.get_by_role("button", name="Refresh schedules", exact=True).click()
            page.get_by_text("Schedule source unavailable", exact=False).wait_for()
            assert page.get_by_role(
                "button", name="Create observation schedule", exact=True
            ).is_disabled()
            forbidden = True
            page.get_by_role("button", name="Refresh schedules", exact=True).click()
            page.get_by_text(
                "The request is outside the permitted scope.", exact=False
            ).wait_for()
            assert identifier not in page.locator("body").inner_text()
            assert "MUST_NOT_RENDER" not in page.content()
            assert not errors, errors
            mutations = [call for call in calls if call["method"] == "POST"]
            assert len(mutations) == 8, mutations
            assert not any(
                call["path"].endswith(("/alerts", "/notifications")) for call in calls
            )
            (args.output / "result.json").write_text(
                json.dumps(
                    {
                        "scope": "closed browser fixture; no real API/Store/host acceptance",
                        "browser": browser.version,
                        "late_disposed_response": late_delivery,
                        "checks": [
                            "mounted despite unavailable Overview",
                            "permission refusal remains visible",
                            "fixed password enablement and reauthentication",
                            "credential input cleared",
                            "already-enabled response retains the current session",
                            "close/reopen disposes a pending credential request",
                            "UTC cross-midnight schedule creation",
                            "revision-checked pause/resume",
                            "unavailable disables writes",
                            "forbidden clears private rows",
                            "no unsupported reads or automatic POST retry",
                            "desktop/mobile render",
                        ],
                        "calls": calls,
                        "page_errors": errors,
                        "live_host_operations": 0,
                    },
                    indent=2,
                )
                + "\n"
            )
        except Exception:
            (args.output / "failure.html").write_text(page.content())
            page.screenshot(path=str(args.output / "failure.png"), full_page=True)
            (args.output / "failure.json").write_text(
                json.dumps({"calls": calls, "errors": errors}, indent=2) + "\n"
            )
            raise
        finally:
            browser.close()


if __name__ == "__main__":
    main()
