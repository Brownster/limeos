#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.11"
# dependencies = ["playwright==1.62.0"]
# ///
"""Mounted policy editor against a closed fixture, never Core or a protected reader."""

import argparse
import hashlib
import json
import mimetypes
import sys
import time
from pathlib import Path
from urllib.parse import urlsplit

from playwright.sync_api import Error, sync_playwright

sys.path.insert(0, str(Path(__file__).resolve().parent))
from browser_checks import fits_viewport, forbid_page_eval  # noqa: E402

ROOT = Path(__file__).resolve().parents[2]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    build = json.loads(
        (ROOT / ".cache/b1-policy-qualification/frontend-build.json").read_text()
    )
    assert build["exit_code"] == 0 and build["inputs_unchanged"]
    for name, sha in {**build["inputs"], **build["ui"]}.items():
        assert hashlib.sha256((ROOT / name).read_bytes()).hexdigest() == sha
    session = {
        "principal": {
            "id": "fixture-operator",
            "role": "operator",
            "grant_revision": 1,
        },
        "csrf_token": "d" * 64,
        "expires_at": int(time.time()) + 3600,
    }
    original_silence = {
        "kind": "mount",
        "key": "mount:/mnt/Media",
        "created_at": "2026-10-20T12:00:00.123456Z",
        "expires_at": None,
        "reason": "Fixture <script>must remain text</script>",
    }
    saved = {
        "revision": 1,
        "updated": 200,
        "definition": {
            "fail_threshold": 2,
            "disabled": [],
            "required_mounts": ["/mnt/Media", "/mnt/media"],
            "silences": [original_silence],
        },
    }
    calls, errors, held, served = [], [], [], {}
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
            nonlocal hold, saved
            request = route.request
            url = urlsplit(request.url)
            assert url.scheme == "https" and url.netloc == "limeos-policy.invalid"
            path = url.path
            if path.startswith("/api/"):
                body = request.post_data_json if request.post_data else None
                calls.append({"path": path, "method": request.method, "body": body})
                status = 200
                if path == "/api/v1/auth/session":
                    data = session
                elif (
                    path == "/api/v1/automation/health-policy"
                    and request.method == "GET"
                ):
                    if hold:
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
                                "message": "Fixture policy unavailable",
                                "retry": True,
                                "audit_id": "fixture-policy",
                            },
                        )
                    else:
                        data = saved
                elif (
                    path == "/api/v1/automation/health-policy"
                    and request.method == "POST"
                ):
                    assert set(body) == {"revision", "definition"}
                    assert request.headers["x-csrf-token"] == session["csrf_token"]
                    assert body["revision"] == saved["revision"]
                    assert body["definition"]["silences"][0] == original_silence
                    saved = {
                        "revision": saved["revision"] + 1,
                        "updated": 201,
                        "definition": body["definition"],
                    }
                    if saved["revision"] == 3:
                        route.fulfill(
                            status=403,
                            content_type="application/json",
                            body=json.dumps(
                                {
                                    "code": "expired",
                                    "message": "Fixture late committed reply",
                                    "retry": False,
                                    "audit_id": "fixture",
                                }
                            ),
                        )
                        return
                    if saved["revision"] == 4:
                        route.abort(
                            "failed"
                        )  # Actual fixture state changed; reply is lost.
                        return
                    data = saved
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
                    raise AssertionError(f"Unexpected request: {request.method} {path}")
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
            page.goto("https://limeos-policy.invalid/")
            # The policy is a sub-page of Alerts, linked from the Overview (was a toggle button).
            button = page.get_by_role("link", name="Manage alert policy", exact=True)
            button.wait_for()
            assert not any("/automation/" in call["path"] for call in calls)
            button.click()
            page.get_by_text(original_silence["reason"], exact=True).wait_for()
            assert (
                page.locator("script").filter(has_text="must remain text").count() == 0
            )
            page.get_by_label(
                "Consecutive failures before an incident", exact=True
            ).fill("3")
            page.get_by_label("Containers", exact=True).uncheck()
            page.get_by_label("Required mount paths", exact=True).fill(
                "/mnt/Media\n/mnt/media\n/Case/New"
            )
            page.get_by_label("Exact alert key", exact=True).fill("mount:/Case/New")
            page.get_by_label("Reason (optional)", exact=True).fill("fixture disk work")
            page.get_by_label("Expiry (optional, with timezone)", exact=True).fill(
                "2026-10-31T07:00:00Z"
            )
            page.get_by_role("button", name="Add silence to draft", exact=True).click()
            assert (
                page.get_by_role("button", name="Remove silence", exact=False).count()
                == 2
            )
            assert not any(call["method"] == "POST" for call in calls)
            page.get_by_role("button", name="Save health settings", exact=True).click()
            page.get_by_text("Saved revision 2", exact=False).wait_for()
            assert saved["definition"]["fail_threshold"] == 3
            assert saved["definition"]["disabled"] == ["container"]
            assert saved["definition"]["required_mounts"] == [
                "/mnt/Media",
                "/mnt/media",
                "/Case/New",
            ]
            assert saved["definition"]["silences"][1]["key"] == "mount:/Case/New"
            page.get_by_label(
                "Consecutive failures before an incident", exact=True
            ).fill("4")
            page.get_by_role("button", name="Save health settings", exact=True).click()
            page.get_by_text("Saved revision 3", exact=False).wait_for()
            page.get_by_text("The save was not confirmed.", exact=False).wait_for()
            assert len([call for call in calls if call["method"] == "POST"]) == 2
            page.get_by_label(
                "Consecutive failures before an incident", exact=True
            ).fill("5")
            page.get_by_role("button", name="Save health settings", exact=True).click()
            page.get_by_text("Saved revision 4", exact=False).wait_for()
            page.get_by_text("The save was not confirmed.", exact=False).wait_for()
            assert len([call for call in calls if call["method"] == "POST"]) == 3
            hold = True
            page.get_by_role(
                "button", name="Refresh health settings", exact=True
            ).click()
            until = time.monotonic() + 5
            while not held:
                assert time.monotonic() < until
                page.wait_for_timeout(10)
            page.get_by_role("link", name="Overview", exact=True).click()
            page.get_by_role("link", name="Manage alert policy", exact=True).click()
            page.get_by_text("Saved revision 4", exact=False).wait_for()
            try:
                held[0].fulfill(status=401, content_type="application/json", body="{}")
                late = "late 401 delivered after owner disposal"
            except Error:
                late = "old transport canceled after owner disposal"
            assert page.locator(".account").is_visible()
            page.screenshot(
                path=str(args.output / "policy-desktop.png"), full_page=True
            )
            page.set_viewport_size({"width": 390, "height": 844})
            assert fits_viewport(page)
            page.screenshot(path=str(args.output / "policy-mobile.png"), full_page=True)
            unavailable = True
            page.get_by_role(
                "button", name="Refresh health settings", exact=True
            ).click()
            page.get_by_text("Fixture policy unavailable", exact=True).wait_for()
            assert page.get_by_role(
                "button", name="Save health settings", exact=True
            ).is_disabled()
            forbidden = True
            page.get_by_role(
                "button", name="Refresh health settings", exact=True
            ).click()
            page.get_by_text(
                "Health scheduling permission is required", exact=False
            ).wait_for()
            assert original_silence["reason"] not in page.locator("body").inner_text()
            assert "MUST_NOT_RENDER" not in page.content()
            assert not any(
                "/schedules" in call["path"] or "/notifications" in call["path"]
                for call in calls
            )
            assert len([call for call in calls if call["method"] == "POST"]) == 3
            assert not errors, errors
            (args.output / "result.json").write_text(
                json.dumps(
                    {
                        "scope": "closed browser fixture only; no live Core/Store, protected collection, external send or cutover acceptance",
                        "browser": browser.version,
                        "late_disposed_response": late,
                        "ui": build["ui"],
                        "served_ui": served,
                        "checks": [
                            "mounted editor with unavailable overview",
                            "literal paths and escaped metadata",
                            "draft-only silence addition",
                            "CAS saves preserve existing creation metadata",
                            "lost reply reconciled by GET without repeat POST",
                            "disposed owner cannot expire replacement",
                            "unavailable disables mutation and forbidden clears data",
                            "desktop/mobile layout",
                            "no scheduler or publication access",
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
