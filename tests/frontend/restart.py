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
                    operation = request.post_data_json.get("operation", "restart")
                    if operation != "restart":
                        data["plan"]["operation"] = operation
                    data["plan"]["expected"]["running"] = (
                        resource["status"] == "running"
                    )
                    status = 201
                elif path.endswith("/logs"):
                    assert request.method == "GET"
                    data = {
                        "resource": resource["id"],
                        "text": 'ready\n<img src=x onerror="window.logExecuted=true">\n[credential-bearing line redacted]\n',
                        "truncated": True,
                    }
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
        assert not page.get_by_role("button", name="Start", exact=True).is_enabled()
        for action, next_status in [("stop", "exited"), ("start", "running")]:
            page.get_by_role("button", name=action.capitalize(), exact=True).click()
            page.get_by_role("button", name=f"Approve and {action}", exact=True).click()
            dialog.wait_for(state="hidden")
            assert jobs[-1]["plan"]["operation"] == action
            jobs[-1]["state"] = "succeeded"
            resource["status"] = next_status
            page.reload()
            page.locator(".job-row").nth(0).get_by_text(
                "succeeded", exact=True
            ).wait_for()
            assert page.get_by_role(
                "button", name="Start", exact=True
            ).is_enabled() == (next_status == "exited")
            assert page.get_by_role("button", name="Stop", exact=True).is_enabled() == (
                next_status == "running"
            )
        mutation_count = len([c for c in calls if c[1] == "POST"])
        log_button = page.get_by_role("button", name="Logs", exact=True)
        log_button.click()
        log_dialog = page.get_by_role("dialog", name="Logs · Test television")
        log_dialog.wait_for(state="visible")
        page.locator(".container-logs").get_by_text("<img", exact=False).wait_for()
        assert page.locator(".container-logs img").count() == 0
        assert not page.evaluate("!!window.logExecuted")
        page.get_by_text("Output reached the size limit.", exact=True).wait_for()
        for _ in range(4):
            page.keyboard.press("Tab")
            assert page.evaluate("!!document.activeElement.closest('dialog')")
        page.get_by_role("button", name="Refresh logs", exact=True).click()
        assert len([c for c in calls if c[0].endswith("/logs")]) == 2
        assert len([c for c in calls if c[1] == "POST"]) == mutation_count
        page.keyboard.press("Escape")
        log_dialog.wait_for(state="hidden")
        assert log_button.evaluate("element => document.activeElement === element")
        evidence = ROOT / "docs/rewrite-evidence/p03"
        evidence.mkdir(parents=True, exist_ok=True)
        page.screenshot(path=str(evidence / "lifecycle-progress.png"), full_page=True)
        session["principal"]["role"] = "viewer"
        page.reload()
        page.get_by_role("heading", name="At home.").wait_for()
        for label in ["Start", "Stop", "Restart", "Logs"]:
            assert page.get_by_role("button", name=label, exact=True).count() == 0
        (evidence / "lifecycle-browser-result.json").write_text(
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
                        "stop and start previews bind their actions and require explicit approval",
                        "lifecycle controls reflect running state",
                        "log HTML is escaped text and clipping is visible",
                        "log refresh uses GET without queueing any effect",
                        "log dialog contains keyboard focus and Escape restores its trigger",
                        "viewer sees neither lifecycle nor log controls",
                    ],
                    "queue_posts": len(
                        [c for c in calls if c[0].endswith("/jobs") and c[1] == "POST"]
                    ),
                    "durable_fixture_jobs": len(jobs),
                },
                indent=2,
            )
            + "\n"
        )
        browser.close()
    print("Browser lifecycle, logs and focus regressions passed.")


if __name__ == "__main__":
    main()
