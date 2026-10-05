#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.11"
# dependencies = ["playwright==1.62.0"]
# ///
"""Browser regression against the built UI and a closed API fixture; no live host."""

import json
import mimetypes
import time
from pathlib import Path

from playwright.sync_api import sync_playwright

ROOT = Path(__file__).resolve().parents[2]


def main():
    identifier = "b" * 64
    resource = {
        "id": "container:" + identifier,
        "kind": "container",
        "name": "Test television",
        "status": "running",
        "source": "docker",
        "image": "fixture:local",
        "health": None,
        "identity": identifier,
        "cpu_percent": None,
        "memory_percent": None,
    }
    proposal = {
        "plan": {
            "id": "c" * 64,
            "version": 1,
            "principal": "test-admin",
            "grant_revision": 1,
            "expected": {
                "resource": resource["id"],
                "image": "sha256:" + "d" * 64,
                "running": True,
                "started_at": "2026-10-05T07:00:00Z",
            },
            "created_at": int(time.time()),
            "expires_at": int(time.time()) + 300,
        },
        "digest": "e" * 64,
    }
    session = {
        "principal": {"id": "test-admin", "role": "administrator", "grant_revision": 1},
        "csrf_token": "a" * 64,
        "expires_at": int(time.time()) + 3600,
    }
    calls = []
    jobs = []
    queued = {}
    previews = 0
    lose_response = False
    with sync_playwright() as playwright:
        browser = playwright.chromium.launch(
            executable_path="/usr/bin/google-chrome",
            headless=True,
            args=["--disable-dev-shm-usage"],
        )
        page = browser.new_page(viewport={"width": 1100, "height": 850})

        def respond(route):
            nonlocal previews, lose_response
            request = route.request
            path = request.url.split("limeos-test.invalid", 1)[1].split("?", 1)[0]
            if path.startswith("/api/"):
                calls.append(
                    (
                        path,
                        request.method,
                        request.post_data_json if request.post_data else None,
                        request.headers,
                    )
                )
                data = {}
                status = 200
                if path.endswith("/auth/session"):
                    data = session
                elif path.endswith("/overview"):
                    data = {
                        "revision": 1,
                        "generated_at": int(time.time()),
                        "host": None,
                        "resources": [resource],
                        "sources": [],
                    }
                elif path.endswith("/system/history"):
                    data = {
                        "points": [],
                        "bucket_seconds": 60,
                        "legacy_history": "No imported history",
                    }
                elif path.endswith("/observations/stream"):
                    status = 503
                elif path.endswith("/plans"):
                    assert (
                        request.method == "POST"
                        and request.headers["x-csrf-token"] == session["csrf_token"]
                    )
                    previews += 1
                    data = json.loads(json.dumps(proposal))
                    data["plan"]["id"] = f"{previews:064x}"
                    status = 201
                elif path.endswith("/approval"):
                    data = {
                        "token": "f" * 64,
                        "expires_at": proposal["plan"]["expires_at"],
                    }
                elif path.endswith("/jobs") and request.method == "POST":
                    body = request.post_data_json
                    key = body["idempotency_key"]
                    if key not in queued:
                        queued[key] = {
                            "id": f"{len(jobs) + 1:064x}",
                            "state": "queued",
                            "plan": body["proposal"]["plan"],
                        }
                        jobs.append(queued[key])
                    data = queued[key]
                    status = 202
                    if lose_response:
                        lose_response = False
                        route.abort("connectionreset")
                        return
                elif path.endswith("/jobs"):
                    data = list(reversed(jobs))
                elif "/jobs/" in path:
                    data = {
                        "job": jobs[0],
                        "events": [
                            {"event": {"kind": "job_transition", "state": "succeeded"}}
                        ],
                    }
                route.fulfill(
                    status=status,
                    content_type="application/json",
                    body=json.dumps(data),
                )
                return
            file = (
                ROOT
                / "frontend/dist"
                / ("index.html" if path == "/" else path.lstrip("/"))
            )
            assert file.is_file()
            route.fulfill(
                body=file.read_bytes(),
                content_type=mimetypes.guess_type(file)[0]
                or "application/octet-stream",
            )

        page.route("https://limeos-test.invalid/**", respond)
        page.goto("https://limeos-test.invalid/")
        restart = page.get_by_role("button", name="Restart", exact=True)
        restart.click()
        dialog = page.get_by_role("dialog")
        dialog.wait_for(state="visible")
        page.get_by_role("button", name="Approve and restart", exact=True).wait_for()
        for _ in range(6):
            page.keyboard.press("Tab")
            assert page.evaluate("!!document.activeElement.closest('dialog')")
        page.keyboard.press("Escape")
        dialog.wait_for(state="hidden")
        assert restart.evaluate("element => document.activeElement === element")
        restart.click()
        page.get_by_role("button", name="Approve and restart", exact=True).click()
        page.get_by_role("heading", name="Recent container operations").wait_for()
        assert len([c for c in calls if c[0].endswith("/jobs") and c[1] == "POST"]) == 1
        jobs[0]["state"] = "succeeded"
        page.get_by_text("succeeded", exact=True).wait_for(timeout=8000)
        page.get_by_role("button", name="Progress", exact=True).click()
        page.locator(".job-events").get_by_text("succeeded").wait_for()
        page.reload()
        page.get_by_text("succeeded", exact=True).wait_for()
        assert len([c for c in calls if c[0].endswith("/jobs") and c[1] == "POST"]) == 1
        approval_count = len([c for c in calls if c[0].endswith("/approval")])
        lose_response = True
        page.get_by_role("button", name="Restart", exact=True).click()
        page.get_by_role("button", name="Approve and restart", exact=True).click()
        retry = page.get_by_role("button", name="Retry same request", exact=True)
        retry.wait_for()
        posts = [c for c in calls if c[0].endswith("/jobs") and c[1] == "POST"]
        assert len(posts) == 2 and len(jobs) == 2
        interrupted_body = posts[-1][2]
        assert (
            len([c for c in calls if c[0].endswith("/approval")]) == approval_count + 1
        )
        retry.click()
        dialog.wait_for(state="hidden")
        posts = [c for c in calls if c[0].endswith("/jobs") and c[1] == "POST"]
        assert len(posts) == 3 and posts[-1][2] == interrupted_body and len(jobs) == 2
        assert (
            len([c for c in calls if c[0].endswith("/approval")]) == approval_count + 1
        )
        jobs[1]["state"] = "succeeded"
        page.locator(".job-row").nth(0).get_by_text("succeeded", exact=True).wait_for()
        evidence = ROOT / "docs/rewrite-evidence/p03"
        evidence.mkdir(parents=True, exist_ok=True)
        page.screenshot(path=str(evidence / "restart-progress.png"), full_page=True)
        (evidence / "browser-result.json").write_text(
            json.dumps(
                {
                    "browser": browser.version,
                    "checks": [
                        "native modal contains keyboard focus",
                        "Escape restores trigger focus",
                        "preview precedes explicit approval",
                        "one POST queues the job",
                        "durable progress survives reload",
                        "lost queue response requires explicit retry with the same plan, approval and key",
                        "explicit retry recovers the same job without another approval",
                    ],
                    "queue_posts": len(posts),
                    "durable_fixture_jobs": len(jobs),
                },
                indent=2,
            )
            + "\n"
        )
        browser.close()
    print("Browser restart and focus regressions passed.")


if __name__ == "__main__":
    main()
