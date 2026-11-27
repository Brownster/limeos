#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.11"
# dependencies = ["playwright==1.62.0"]
# ///
"""Exercise the mounted Stacks screen against closed read-only API fixtures."""

import argparse
import json
import mimetypes
import time
from pathlib import Path

from playwright.sync_api import sync_playwright

ROOT = Path(__file__).resolve().parents[2]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    value = json.loads(
        (ROOT / "tests/fixtures/deployment/discovery/wire.json").read_text()
    )
    at = int(time.time())
    value["generated_at"] = at
    for source in ("catalog", "engine"):
        value[source].update(sampled_at=at, valid_for_millis=90_000)
    calls = []
    forbidden = False
    session = {
        "principal": {"id": "reader", "role": "viewer", "grant_revision": 1},
        "csrf_token": "a" * 64,
        "expires_at": at + 3600,
    }
    with sync_playwright() as playwright:
        browser = playwright.chromium.launch(
            executable_path="/usr/bin/google-chrome",
            headless=True,
            args=["--disable-dev-shm-usage"],
        )
        page = browser.new_page(viewport={"width": 1100, "height": 900})
        errors = []
        page.on("pageerror", lambda error: errors.append(str(error)))

        def respond(route):
            request = route.request
            path = request.url.split("limeos-stacks.invalid", 1)[1].split("?", 1)[0]
            if path.startswith("/api/"):
                assert request.method == "GET", "stack fixture accepts no effect"
                calls.append(path)
                status = 200
                if path == "/api/v1/auth/session":
                    data = session
                elif path == "/api/v1/deployment/stacks":
                    status = 403 if forbidden else 200
                    data = {"private": "MUST_NOT_RENDER"} if forbidden else value
                else:
                    # Stacks remain reachable when general observations/history
                    # are outside this reader's scope or their sources fail.
                    status = 503
                    data = {
                        "code": "unavailable",
                        "message": "Observations unavailable",
                        "audit_id": "fixture",
                    }
                route.fulfill(
                    status=status,
                    content_type="application/json",
                    body=json.dumps(data),
                )
                return
            asset = (
                ROOT
                / "frontend/dist"
                / ("index.html" if path == "/" else path.lstrip("/"))
            )
            assert asset.resolve().is_relative_to((ROOT / "frontend/dist").resolve())
            route.fulfill(
                status=200,
                content_type=mimetypes.guess_type(str(asset))[0]
                or "application/octet-stream",
                body=asset.read_bytes(),
            )

        page.route("https://limeos-stacks.invalid/**", respond)
        page.goto("https://limeos-stacks.invalid/")
        timer_probe = page.evaluate("""() => {
          try { const clock = { schedule: window.setTimeout }; clock.schedule(() => {}, 1); return "accepted"; }
          catch (error) { return error.name + ": " + error.message; }
        }""")
        assert "Illegal invocation" in timer_probe
        page.get_by_role("link", name="Stacks", exact=True).click()
        panel = page.locator('section[aria-labelledby="stack-management-heading"]')
        panel.get_by_role("heading", name="Stacks and services").wait_for()
        try:
            panel.get_by_text(
                "1 of 2 service containers running", exact=False
            ).first.wait_for()
        except Exception:
            (args.output / "failure.html").write_text(page.content())
            (args.output / "failure.json").write_text(
                json.dumps(
                    {
                        "calls": calls,
                        "errors": errors,
                        "fixture": value,
                        "browser_clock": page.evaluate("Date.now() / 1000"),
                    },
                    indent=2,
                )
                + "\n"
            )
            page.screenshot(path=str(args.output / "failure.png"), full_page=True)
            raise
        panel.locator("summary").filter(has_text="Services and members").click()
        assert panel.locator("tbody tr").count() == 3
        assert "One-off" in panel.inner_text()
        assert "/opt/stacks/Media" in panel.inner_text()
        page.screenshot(path=str(args.output / "stacks.png"), full_page=True)
        for source in ("engine",):
            value[source].update(
                state="unavailable", sampled_at=None, valid_for_millis=0
            )
        value["engine_matches_catalog"] = None
        value["standalone_containers"] = None
        for stack in value["stacks"]:
            stack.update(runtime="unknown", counts=None, missing_services=None)
            for service in stack["services"]:
                service.update(runtime="unknown", counts=None, members=[])
        panel.get_by_role("button", name="Refresh stacks", exact=True).click()
        panel.get_by_text(
            "Current container counts are unknown.", exact=True
        ).first.wait_for()
        assert "1 of 2 service containers running" not in panel.inner_text()
        forbidden = True
        panel.get_by_role("button", name="Refresh stacks", exact=True).click()
        panel.get_by_text(
            "You no longer have access to stack inventory.", exact=True
        ).wait_for()
        assert "/opt/stacks/Media" not in panel.inner_text()
        assert "MUST_NOT_RENDER" not in page.content()
        assert calls.count("/api/v1/deployment/stacks") == 3
        (args.output / "result.json").write_text(
            json.dumps(
                {
                    "browser": browser.version,
                    "checks": [
                        "native browser timer binding",
                        "mounted view despite failed overview",
                        "replicas, stopped member and one-off",
                        "source failure withholds counts",
                        "forbidden clears private facts",
                        "fixed GETs only",
                    ],
                    "unbound_browser_timer": timer_probe,
                    "stack_reads": 3,
                    "live_host_operations": 0,
                },
                indent=2,
            )
            + "\n"
        )
        browser.close()


if __name__ == "__main__":
    main()
