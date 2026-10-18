"""Exercise the real closed browser fixture without a browser or sockets."""

import json
import runpy
import sys
import unittest
from pathlib import Path
from types import ModuleType, SimpleNamespace
from unittest.mock import patch

SOURCE = Path(__file__).resolve().parents[1] / "frontend/restart.py"


class StopBeforeBrowserActions(Exception):
    pass


class FixtureRoute:
    def __init__(self, path, method="GET", body=None):
        self.request = SimpleNamespace(
            url="https://limeos-test.invalid" + path,
            method=method,
            post_data=json.dumps(body) if body is not None else None,
            post_data_json=body,
            headers={"x-csrf-token": "a" * 64},
        )
        self.response = None

    def fulfill(self, **response):
        self.response = response


class FixturePage:
    def route(self, pattern, handler):
        assert pattern == "https://limeos-test.invalid/**"
        self.handler = handler

    def goto(self, url):
        assert url == "https://limeos-test.invalid/"
        raise StopBeforeBrowserActions


class FixturePlaywright:
    def __init__(self, page):
        self.page = page

    def __enter__(self):
        browser = SimpleNamespace(new_page=lambda **_: self.page)
        return SimpleNamespace(chromium=SimpleNamespace(launch=lambda **_: browser))

    def __exit__(self, *_):
        return False


class LifecycleBrowserFixtureTests(unittest.TestCase):
    def setUp(self):
        page = FixturePage()
        sync_api = ModuleType("playwright.sync_api")
        sync_api.sync_playwright = lambda: FixturePlaywright(page)
        with patch.dict(
            sys.modules,
            {"playwright": ModuleType("playwright"), "playwright.sync_api": sync_api},
        ):
            namespace = runpy.run_path(str(SOURCE))
            with self.assertRaises(StopBeforeBrowserActions):
                namespace["main"]()
        self.handler = page.handler

    def response(self, path, method="GET", body=None):
        route = FixtureRoute(path, method, body)
        self.handler(route)
        self.assertIsNotNone(route.response)
        return route.response["status"], json.loads(route.response["body"])

    def test_mount_catalog_has_typed_empty_targets(self):
        self.assertEqual(
            self.response("/api/v1/storage/mounts/targets"), (200, {"targets": []})
        )

    def test_mount_history_never_reuses_container_jobs(self):
        body = {
            "proposal": {
                "plan": {
                    "id": "c" * 64,
                    "expected": {"resource": "container:" + "b" * 64},
                }
            },
            "idempotency_key": "fixture-container-request",
        }
        status, job = self.response("/api/v1/container/jobs", "POST", body)
        self.assertEqual(status, 202)
        self.assertEqual(self.response("/api/v1/container/jobs"), (200, [job]))
        self.assertEqual(self.response("/api/v1/storage/mounts/jobs"), (200, []))

    def test_unexpected_mount_actions_are_refused_before_fixture_mutation(self):
        for path, method in (
            ("/api/v1/storage/mounts/targets", "POST"),
            ("/api/v1/storage/mounts/jobs", "POST"),
            ("/api/v1/storage/mounts/plans", "POST"),
            ("/api/v1/storage/mounts/unknown", "GET"),
        ):
            with self.subTest(path=path, method=method):
                route = FixtureRoute(path, method)
                with self.assertRaises(AssertionError):
                    self.handler(route)
                self.assertIsNone(route.response)
        self.assertEqual(self.response("/api/v1/container/jobs"), (200, []))


if __name__ == "__main__":
    unittest.main()
