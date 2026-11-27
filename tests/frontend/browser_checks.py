"""CSP-safe browser checks shared by the tests in tests/frontend.

The UI is served under `script-src 'self'` without 'unsafe-eval'. Playwright
(1.62) runs page.evaluate, locator.evaluate, evaluate_all, evaluate_handle
and wait_for_function by calling `globalThis.eval` in the page's main world,
which such a CSP forbids, so those calls fail in a browser that enforces it.
Locator actions, expectations and queries run in Playwright's isolated
utility world and are fine. Use these helpers instead of page evaluation;
the Chrome DevTools Protocol helpers below read browser state without
evaluating a string in the page.
"""

from playwright.sync_api import Page

# Some Chrome builds let DevTools-initiated eval through a page's CSP, so a
# stray page.evaluate passes locally and fails in CI. This init script makes
# every main-world eval fail everywhere, the way an enforcing browser does.
# The app never uses eval (its CSP forbids it), so only test code can trip it.
FORBID_PAGE_EVAL = """
Object.defineProperty(globalThis, "eval", {
  value: function () {
    throw new EvalError("page-world eval is forbidden in these tests: the UI's CSP has no 'unsafe-eval'; use tests/frontend/browser_checks.py");
  },
});
"""


def forbid_page_eval(page: Page) -> None:
    page.add_init_script(FORBID_PAGE_EVAL)


def _cdp(page: Page, method: str, params: dict | None = None) -> dict:
    session = page.context.new_cdp_session(page)
    try:
        return session.send(method, params or {})
    finally:
        session.detach()


def horizontal_overflow(page: Page) -> float:
    """CSS pixels by which the document is wider than the viewport (0 when it fits)."""
    metrics = _cdp(page, "Page.getLayoutMetrics")
    content = metrics["cssContentSize"]["width"]
    viewport = metrics["cssLayoutViewport"]["clientWidth"]
    return max(0.0, content - viewport)


def fits_viewport(page: Page) -> bool:
    return horizontal_overflow(page) <= 1


def cdp_value(page: Page, expression: str):
    """Read one value from the page through DevTools (not page eval).

    For probes that only JavaScript can answer (a global the page must not
    have set, a native binding). Prefer locators for anything in the DOM.
    """
    reply = _cdp(page, "Runtime.evaluate", {"expression": expression, "returnByValue": True})
    if "exceptionDetails" in reply:
        raise AssertionError(reply["exceptionDetails"].get("exception", {}).get("description", reply["exceptionDetails"]))
    return reply["result"].get("value")


def focus_within(page: Page, scope: str) -> bool:
    """Whether keyboard focus is on `scope` or inside it (a CSS selector)."""
    return page.locator(f"{scope}:focus, {scope} :focus").count() == 1


def scrolls_horizontally(page: Page, region, marker) -> bool:
    """Whether `region` scrolls sideways: wheel over it and watch `marker` move."""
    before = marker.bounding_box()["x"]
    region.hover()
    page.mouse.wheel(2000, 0)
    page.wait_for_timeout(150)
    moved = marker.bounding_box()["x"] < before - 1
    page.mouse.wheel(-2000, 0)
    return moved
