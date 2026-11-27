#!/usr/bin/env python3
"""Real-browser qualification of the job-resolution UI; exclusively inside the disposable Debian VM.

Induces needs_intervention jobs exactly like p04_resolution_guest (container restart, failing starts, stack
restart/start), then drives the packaged, Caddy-served frontend with headless Chromium (Selenium, as the other
browser guests do) as a real administrator and as an operator: banner, dialog focus, plan-vs-measured evidence,
disabled outcomes, divergence, wrong password, stale evidence, success, pending (executor stopped), keyboard-only
operation and credential hygiene. Each case records pass/fail and continues; screenshots are embedded in the result.
"""

import base64
import hashlib
import json
import os
import signal
import socket
import sqlite3
import sys
import threading
import time
import traceback
from pathlib import Path
from urllib.parse import urlsplit

import p04_automation_browser_guest as browser
import p04_resolution_guest as rg

sl, fx, check = rg.sl, rg.fx, rg.check
PASSWORD, ADMIN, OPERATOR = rg.PASSWORD, rg.ADMIN, "operator-res"
WRONG = "Wrong-" + hashlib.sha256(os.urandom(8)).hexdigest()[:20]
DIALOG = "dialog.resolution-dialog[open]"
results, evidence = {}, {"screenshots": {}, "console": [], "a11y": [], "network": [], "notes": []}
driver = None
order = []
SECRETS = (PASSWORD, WRONG)


# ---------------------------------------------------------------- harness
def case(name):
    def decorate(function):
        def run():
            started = time.monotonic()
            try:
                detail = function() or {}
                results[name] = {"status": "pass", "detail": detail}
                fx.passed(name)
            except BaseException as error:  # noqa: BLE001 - recorded, never hidden
                results[name] = {"status": "fail", "error": repr(error)[:2500], "trace": traceback.format_exc()[-3000:]}
                print("FAIL " + name + ": " + repr(error)[:1500], flush=True)
                try:
                    shot("FAIL-" + name.split()[0])
                    results[name]["page_text"] = driver.execute_script("return document.body.innerText")[:2500]
                except Exception:  # noqa: BLE001
                    pass
            finally:
                results[name]["seconds"] = round(time.monotonic() - started, 1)
                drain()
                try:
                    fx.wait_container_ready(timeout=45)
                    sl.wait_core()
                except Exception as error:  # noqa: BLE001
                    results[name]["service_recovery_error"] = repr(error)[:500]

        order.append(run)
        return function

    return decorate


requests = {}


def drain():
    """Collect console and network logs; fail-safe scans run on the accumulated copies."""
    for entry in driver.get_log("browser"):
        evidence["console"].append({"level": entry["level"], "message": entry["message"][:400], "t": time.time()})
    for entry in driver.get_log("performance"):
        message = json.loads(entry["message"])["message"]
        event, fields = message["method"], message.get("params", {})
        if event == "Network.requestWillBeSent":
            url = urlsplit(fields["request"]["url"])
            requests[fields["requestId"]] = {"method": fields["request"]["method"], "url": url.path + ("?" + url.query if url.query else ""),
                                              "headers": sorted(k.lower() for k in fields["request"].get("headers", {}))}
        elif event == "Network.responseReceived":
            row = requests.get(fields["requestId"])
            if row is not None:
                row["status"] = fields["response"]["status"]
                evidence["network"].append(row)
        # postData is deliberately never retained


def wait_for(observe, timeout=20):
    return browser.wait_for(observe, timeout=timeout)


def js(script, *args):
    return driver.execute_script(script, *args)


def find(selector):
    return driver.find_element("css selector", selector)


def finds(selector):
    return driver.find_elements("css selector", selector)


def ready(selector, text=None, timeout=20):
    def observed():
        try:
            for value in finds(selector):
                if value.is_displayed() and (text is None or text in value.text):
                    return value
        except Exception:  # noqa: BLE001 - stale element while React re-renders
            return None
        return None

    return wait_for(observed, timeout)


def gone(selector, text=None, timeout=20):
    def observed():
        try:
            return not any(v.is_displayed() and (text is None or text in v.text) for v in finds(selector))
        except Exception:  # noqa: BLE001
            return False

    return wait_for(observed, timeout)


def click_button(label, scope="body", exact=True):
    def observed():
        try:
            matches = [b for b in finds(scope + " button")
                       if b.is_displayed() and b.is_enabled() and ((b.text.replace("↗", "").strip() == label) if exact else label in b.text)]
        except Exception:  # noqa: BLE001
            return None
        return matches[0] if len(matches) == 1 else None

    wait_for(observed).click()


def shot(name, focus=None):
    """Viewport screenshot; `focus` is a selector inside the dialog to scroll to the middle first."""
    if focus:
        js("const e=document.querySelector(arguments[0]);if(e)e.scrollIntoView({block:'center'})", DIALOG + " " + focus)
    require_empty_password()
    data = driver.get_screenshot_as_png()
    check(data[:8] == b"\x89PNG\r\n\x1a\n" and len(data) < 900 * 1024, "screenshot invalid or too large")
    evidence["screenshots"][name] = {"sha256": hashlib.sha256(data).hexdigest(), "png_base64": base64.b64encode(data).decode()}


def require_empty_password():
    values = js("return [...document.querySelectorAll('input[type=password]')].map(e=>e.value)")
    check(all(v == "" for v in values), "password field not empty before screenshot")


def active():
    return js(
        """const a=document.activeElement, d=document.querySelector('dialog.resolution-dialog[open]');
        const label=(a.getAttribute&&a.getAttribute('aria-label'))||'';
        return {tag:a.tagName.toLowerCase(), id:a.id||'', type:a.type||'', label:label,
                text:((a.innerText||'')+'').trim().slice(0,70), inDialog:!!(d&&d.contains(a)), body:a===document.body}"""
    )


def press(*keys):
    from selenium.webdriver import ActionChains

    chain = ActionChains(driver)
    for key in keys:
        chain.send_keys(key)
    chain.perform()


def type_text(text):
    from selenium.webdriver import ActionChains

    ActionChains(driver).send_keys(text).perform()


def tab_until(predicate, limit=80, key=None):
    """Press Tab (or `key`) until the focused element satisfies the predicate; returns the Tab count."""
    from selenium.webdriver.common.keys import Keys

    for count in range(limit + 1):
        if predicate(active()):
            return count
        press(key or Keys.TAB)
    raise AssertionError("keyboard focus never reached the target; last=" + json.dumps(active()))


def secret_scan(label):
    """No credential in the DOM, storage, console, or any request URL/header name seen so far."""
    drain()
    dom = js("return document.documentElement.outerHTML")
    values = js("return [...document.querySelectorAll('input,textarea')].map(e=>e.value)")
    storage = js("return JSON.stringify([Object.entries(localStorage),Object.entries(sessionStorage)])+document.cookie")
    for secret in SECRETS:
        check(secret not in dom, f"{label}: secret in DOM")
        check(all(secret not in v for v in values), f"{label}: secret in a field value")
        check(secret not in storage, f"{label}: secret in storage/cookie script view")
        check(all(secret not in e["message"] for e in evidence["console"]), f"{label}: secret in console")
        check(all(secret not in r["url"] for r in evidence["network"]), f"{label}: secret in a request URL")
    check(all("password" not in r["url"].lower() for r in evidence["network"]), f"{label}: password in a request URL")
    check(all("authorization" not in r["headers"] and "x-api-key" not in r["headers"] for r in evidence["network"] if "/resolution" in r["url"]),
          f"{label}: bearer credentials on a resolution request")


def login(user, password=PASSWORD):
    rg.pace()
    driver.get("https://localhost/")
    ready("#username").send_keys(user)
    find("#password").send_keys(password)
    click_button("Sign in")
    ready(browser.PAGE_TITLE)


def logout():
    click_button("Sign out")
    ready("h1", "Welcome home.")


def account_text():
    """'<principal> · <role>', as the old header's account line read."""
    return js("const a=document.querySelector('.account');return a?a.querySelector('.account-name').textContent+' · '+a.querySelector('.account-role').textContent:''")


def home(user=ADMIN):
    """Reload the app as `user` (logging in when the page shows the sign-in form), on the Containers page.

    The old home page held the container operations list; in the app shell it is on /containers."""
    driver.get("https://localhost/")
    state = wait_for(lambda: "in" if finds(browser.PAGE_TITLE) else ("out" if finds("#username") else None))
    if state == "in" and user not in account_text():
        logout()
        state = "out"
    if state == "out":
        login(user)
    container_list()


def stacks_view():
    if js("return location.pathname") != "/stacks":
        browser.follow(driver, "Stacks")
    ready("section[aria-labelledby='stack-operations-heading']")


def container_list():
    if js("return location.pathname") != "/containers":
        browser.follow(driver, "Containers")
    return ready("section[aria-labelledby='operations-heading']")


def stack_list():
    return ready("section[aria-labelledby='stack-operations-heading']")


def attention(list_selector):
    return ready(list_selector + " section.attention")


def key_of(facts):
    return facts["ids"][0][:12] if facts["kind"] == "container" else facts["project"]


def banner_button(facts):
    scope = "section[aria-labelledby='operations-heading']" if facts["kind"] == "container" else "section[aria-labelledby='stack-operations-heading']"
    key = key_of(facts)

    def observed():
        for b in finds(scope + " section.attention button"):
            if key in (b.get_attribute("aria-label") or "") and b.is_displayed():
                return b
        return None

    return wait_for(observed)


def open_dialog(facts, keyboard=False):
    if facts["kind"] == "stack":
        stacks_view()
    else:
        container_list()
    button = banner_button(facts)
    label = button.get_attribute("aria-label")
    if keyboard:
        from selenium.webdriver.common.keys import Keys

        js("document.activeElement.blur(); window.scrollTo(0,0)")
        tab_until(lambda a: a["label"] == label, limit=200)
        press(Keys.ENTER)
    else:
        button.click()
    ready(DIALOG)
    wait_for(lambda: "Inspecting" not in find(DIALOG).text and finds(DIALOG + " .resolution-target"))
    return label


def close_dialog():
    click_button("Close", scope=DIALOG)
    gone(DIALOG)


def choose(declared):
    radio = find(f"{DIALOG} input[name=declared][value={declared}]")
    check(radio.is_enabled(), "radio disabled: " + declared)
    radio.click()
    ready(DIALOG + " #resolution-reason")


def set_reason(text):
    from selenium.webdriver.common.keys import Keys

    box = find(DIALOG + " #resolution-reason")
    box.click()
    box.send_keys(Keys.CONTROL, "a")  # one call, so the modifier is released again
    box.send_keys(Keys.DELETE)
    box.send_keys(text)


def review():
    click_button("Review decision", scope=DIALOG)


def submit_password(password, label="Resolve job"):
    box = ready(DIALOG + " #resolution-password")
    box.click()
    box.send_keys(password)
    rg.pace()
    click_button(label, scope=DIALOG)


def dialog_text():
    return find(DIALOG).text


def attention_count(list_selector):
    text = js("const s=document.querySelector(arguments[0]+' section.attention h3');return s?s.textContent:''", list_selector)
    return int(text.split("·")[1].split()[0]) if "·" in text else 0


def row_state(facts):
    scope = "section[aria-labelledby='operations-heading']" if facts["kind"] == "container" else "section[aria-labelledby='stack-operations-heading']"
    key = key_of(facts)
    return js(
        """const rows=[...document.querySelectorAll(arguments[0]+' article')].filter(r=>r.textContent.includes(arguments[1]));
        return rows.map(r=>r.innerText.replace(/\\s+/g,' ').trim())""",
        scope, key)


def resolve_via_ui(facts, declared, reason, window_events=True):
    """Choose, review, confirm and submit with the right password; asserts the Engine saw nothing."""
    choose(declared)
    set_reason(reason)
    review()
    ready(DIALOG + " #resolution-password")
    mark = sl.mark()
    submit_password(PASSWORD)
    ready(DIALOG + " #resolved-heading")
    time.sleep(3)
    check(rg.lifecycle_events(mark) == [], "resolution caused Engine events")
    check(rg.core_job(facts["job"]) == "resolved", "core job not resolved", state=rg.core_job(facts["job"]))
    check(rg.job_locks(facts["job"]) == [], "locks remain")


# ---------------------------------------------------------------- cases
@case("1 banner for an administrator: container jobs list and stack jobs list")
def banner_admin():
    home(ADMIN)
    container_list()
    section = attention("section[aria-labelledby='operations-heading']")
    text = section.text
    check("Needs attention" in text and "unknown outcome" in text and "Review and resolve" in text, "container banner text", text=text)
    n_stuck = attention_count("section[aria-labelledby='operations-heading']")
    labels = [b.get_attribute("aria-label") for b in finds("section[aria-labelledby='operations-heading'] section.attention button")]
    check(n_stuck == len(labels) == 3 and any(rg.jobs["H"]["ids"][0][:12] in l for l in labels), "container banner entries", n=n_stuck, labels=labels)
    check("Ask an administrator" not in text, "admin sees the non-admin hint")
    check(rg.jobs["N5"]["ids"][0][:12] not in section.text, "admin unexpectedly lists the operator's job")
    evidence["notes"].append("FINDING: job lists are per principal (jobs WHERE principal=?): the administrator's banner does not list needs_intervention jobs queued by an operator, so the UI cannot reach them; only limeosctl by job id can.")
    shot("01-banner-containers-admin")
    stacks_view()
    section = attention("section[aria-labelledby='stack-operations-heading']")
    labels = [b.get_attribute("aria-label") for b in finds("section[aria-labelledby='stack-operations-heading'] section.attention button")]
    check(len(labels) == 2 and any("sl2" in l for l in labels) and any("sl4" in l for l in labels), "stack banner entries", labels=labels)
    shot("02-banner-stacks-admin")
    secret_scan("banner")
    return {"container_banner": n_stuck, "stack_banner_buttons": labels}


@case("2 operator sees read-only text without the button (containers and stacks)")
def banner_operator():
    home(OPERATOR)
    account = account_text()
    check("operator" in account, "not signed in as operator", account=account)
    container_list()
    section = attention("section[aria-labelledby='operations-heading']")
    check("Ask an administrator to review them." in section.text and "Needs attention · 1 operation" in section.text and rg.jobs["N5"]["ids"][0][:12] in section.text, "operator banner text", text=section.text)
    check(not finds("section.attention button") and "Review and resolve" not in js("return document.body.innerText"), "operator sees a resolve button")
    check(not finds("dialog.resolution-dialog"), "operator has a dialog element")
    shot("03-banner-containers-operator")
    stacks_view()
    section = attention("section[aria-labelledby='stack-operations-heading']")
    check("Ask an administrator" in section.text and not finds("section.attention button"), "operator stack banner", text=section.text)
    check(attention_count("section[aria-labelledby='stack-operations-heading']") == 1 and rg.jobs["N6"]["project"] in section.text, "operator stack entry", text=section.text)
    shot("04-banner-stacks-operator")
    secret_scan("operator")
    logout()
    return {"account": account}


@case("3 dialog: focus moves in, is contained, returns to the opener on Close and on Escape; accessible names")
def dialog_focus():
    from selenium.webdriver.common.keys import Keys

    home(ADMIN)
    h = rg.jobs["H"]
    label = open_dialog(h)
    first = active()
    check(first["inDialog"], "focus did not move into the dialog", active=first)
    modal = js("const d=document.querySelector(arguments[0]);return {open:d.open, modal:d.matches(':modal'), name:d.getAttribute('aria-labelledby')}", DIALOG)
    check(modal["modal"], "dialog is not modal", modal=modal)
    check(js("return !!document.getElementById(arguments[0])", modal["name"]), "aria-labelledby target missing")
    shot("05-dialog-open-container")
    seen = []
    for _ in range(40):  # more tab stops than the dialog has: focus must wrap inside it
        press(Keys.TAB)
        seen.append(active())
    check(all(a["inDialog"] for a in seen), "Tab escaped the dialog", escaped=[a for a in seen if not a["inDialog"]][:3])
    for _ in range(40):
        press(Keys.SHIFT, Keys.TAB)
        a = active()
        check(a["inDialog"], "Shift+Tab escaped the dialog", active=a)
    # accessible-name audit
    problems = js(
        """const d=document.querySelector(arguments[0]);const out=[];
        for(const e of d.querySelectorAll('button,input,textarea,select')){
          const id=e.id; const lab=id&&document.querySelector('label[for="'+CSS.escape(id)+'"]');
          const name=(e.getAttribute('aria-label')||'')||(e.labels&&e.labels.length?e.labels[0].textContent:'')||(lab?lab.textContent:'')||e.textContent;
          if(!name.trim()) out.push(e.outerHTML.slice(0,120));
          for(const idr of (e.getAttribute('aria-describedby')||'').split(' ').filter(Boolean)) if(!document.getElementById(idr)) out.push('dangling describedby '+idr);}
        return out""", DIALOG)
    check(problems == [], "unnamed controls or dangling references", problems=problems)
    close_dialog()
    returned = active()
    check(returned["label"] == label, "focus did not return to the opener after Close", active=returned, expected=label)
    open_dialog(h)
    press(Keys.ESCAPE)
    gone(DIALOG)
    returned = active()
    check(returned["label"] == label, "focus did not return to the opener after Escape", active=returned, expected=label)
    secret_scan("focus")
    return {"initial_focus": first, "tab_stops_seen": len({json.dumps(a, sort_keys=True) for a in seen})}


@case("4 per-target plan vs measured evidence (container and multi-container stack)")
def plan_vs_measured():
    home(ADMIN)
    detail = {}
    for label in ("H", "D3"):
        facts = rg.jobs[label]
        open_dialog(facts)
        view = rg.fresh_view(facts)
        rg.check_view(facts, view)  # API view equals independent Engine inspection
        articles = finds(DIALOG + " article.resolution-target")
        check(len(articles) == len(view["targets"]) == len(facts["ids"]), "target count", n=len(articles))
        for article, target in zip(articles, view["targets"]):
            text = article.text
            check(text.count("Plan expected") == 1 and text.count("Measured now") == 1, "plan/measured headings", text=text)
            check(target["measured"]["container"][:12] in text, "short container id missing")
            for field in ("started_at", "image"):
                check(target["expected"][field] in text and target["measured"][field] in text, "evidence field not shown: " + field, text=text[:400])
            check(("Same as the plan." if target["measured"]["matches_expected"] else "Differs from the plan.") in text, "match sentence", text=text)
            check(("The intended end state is present." if target["measured"]["intended_state_reached"] else "The intended end state is not present.") in text, "reached sentence")
        if label == "D3":
            check("What the executor recorded" in dialog_text(), "executor member outcomes missing")
            shot("06-evidence-stack-three-targets")
        else:
            shot("06a-evidence-container")
        detail[label] = {"targets": len(articles), "allowed": view["allowed"]}
        close_dialog()
    return detail


@case("5 disallowed outcomes are disabled with an explanation")
def disabled_outcomes():
    home(ADMIN)
    detail = {}
    expectations = {"H": {"verified_done": True, "not_done": False, "accepted_divergence": True},
                    "D3": {"verified_done": False, "not_done": False, "accepted_divergence": True},
                    "N1": {"verified_done": False, "not_done": True, "accepted_divergence": True}}
    for label, wanted in expectations.items():
        facts = rg.jobs[label]
        open_dialog(facts)
        for declared, enabled in wanted.items():
            radio = find(f"{DIALOG} input[name=declared][value={declared}]")
            check(radio.is_enabled() == enabled, f"{label}.{declared} enabled state", enabled=radio.is_enabled())
            why = js("return document.getElementById(arguments[0].getAttribute('aria-describedby')).textContent", radio)
            if enabled:
                check(why and "Not available" not in why, "enabled outcome lacks its meaning", why=why)
            else:
                check(why.startswith("Not available:") and facts["ids"][0][:12] in why or "Not available:" in why, "explanation missing", why=why)
            detail[f"{label}.{declared}"] = {"enabled": radio.is_enabled(), "why": why[:140]}
        if label in ("D3", "N1"):
            shot("07-disabled-outcomes-" + label.lower(), ".outcomes")
        close_dialog()
    return detail


@case("6 divergence: >=20 character reason enforced; confirmation is visually distinct")
def divergence():
    home(ADMIN)
    d3 = rg.jobs["D3"]
    open_dialog(d3)
    choose("accepted_divergence")
    help_label = find(DIALOG + " label[for=resolution-reason]").text
    check("at least 20" in help_label, "20-character rule not disclosed", label=help_label)
    set_reason("too short reason")  # 16 characters
    review()
    alert = ready(DIALOG + " p.error[role=alert]").text
    check("at least 20 characters (%d written)" % len("too short reason") in alert, "short reason message", alert=alert)
    check(not finds(DIALOG + " #resolution-password") and find(DIALOG + " #resolution-reason").get_attribute("aria-invalid") == "true", "short reason reached confirmation")
    shot("08-divergence-short-reason", "p.error")
    set_reason("Checked the host: members differ by hand")
    review()
    ready(DIALOG + " #resolution-password")
    summary = find(DIALOG + " .summary")
    check("divergence" in summary.get_attribute("class"), "divergence summary not marked", cls=summary.get_attribute("class"))
    check("accepting a state that differs" in summary.text, "divergence warning text")
    danger = find(DIALOG + " button[type=submit]").get_attribute("class")
    check("danger" in danger, "submit not styled as dangerous", cls=danger)
    style_div = js("const s=getComputedStyle(document.querySelector(arguments[0]+' .summary'));return {bg:s.backgroundColor,border:s.borderColor,w:s.borderWidth}", DIALOG)
    shot("09-divergence-confirm", ".summary")
    # compare with an ordinary confirmation on another job
    close_dialog()
    h = rg.jobs["H"]
    open_dialog(h)
    choose("verified_done")
    set_reason("Container restarted at the expected time")
    review()
    ready(DIALOG + " #resolution-password")
    summary = find(DIALOG + " .summary")
    check("divergence" not in summary.get_attribute("class"), "ordinary summary marked divergent")
    style_plain = js("const s=getComputedStyle(document.querySelector(arguments[0]+' .summary'));return {bg:s.backgroundColor,border:s.borderColor,w:s.borderWidth}", DIALOG)
    check(style_div != style_plain, "divergence confirmation is not visually distinct", div=style_div, plain=style_plain)
    plain_btn = js("const b=document.querySelector(arguments[0]+' button[type=submit]');const s=getComputedStyle(b);return {bg:s.backgroundColor,c:s.color}", DIALOG)
    shot("10-ordinary-confirm", ".summary")
    # leave the verified_done confirmation for case 7 on a fresh load
    close_dialog()
    return {"divergence_style": style_div, "plain_style": style_plain, "plain_button": plain_btn}


@case("7 wrong password: right message, field cleared, nothing recorded, session intact")
def wrong_password():
    home(ADMIN)
    h = rg.jobs["H"]
    open_dialog(h)
    choose("verified_done")
    set_reason("Container restarted at the expected time")
    review()
    mark = sl.mark()
    submit_password(WRONG)
    alert = ready(DIALOG + " p.error[role=alert]", "password was not accepted").text
    check("Nothing was recorded." in alert, "message lacks the reassurance", alert=alert)
    field = find(DIALOG + " #resolution-password")
    check(field.get_attribute("value") == "", "password field not cleared")
    check(rg.core_job(h["job"]) == "needs_intervention" and rg.resolution_row(h["job"]) == [], "wrong password changed state")
    check("Confirm with your password" in dialog_text() and finds("h1"), "dialog or session lost after wrong password")
    session = driver.execute_async_script("const cb=arguments[arguments.length-1];fetch('/api/v1/auth/session',{credentials:'same-origin'}).then(r=>cb(r.status))")
    check(session == 200, "session lost after a wrong password", status=session)
    secret_scan("wrong-password")
    shot("11-wrong-password", "p.error")
    check(rg.lifecycle_events(mark) == [], "wrong password caused Engine events")
    statuses = [r["status"] for r in evidence["network"] if r["method"] == "POST" and r["url"].endswith("/resolution")]
    check(statuses[-1] == 401, "POST status", statuses=statuses)
    close_dialog()
    return {"alert": alert, "post_statuses": statuses}


@case("8 stale evidence: background change, submit refreshes the view and clears the confirmation")
def stale():
    home(ADMIN)
    n3 = rg.jobs["N3"]
    open_dialog(n3)
    choose("not_done")
    set_reason("Nothing started, container still created")
    review()
    ready(DIALOG + " #resolution-password")
    before = rg.fresh_view(n3)["evidence_digest"]
    # the world changes while the confirmation is open: fix the cause and start the container
    rg.make_runnable(rg.fixture["bad_dirs"][n3["solo"]])
    fx.docker("start", n3["ids"][0])
    wait_for(lambda: rg.fresh_view(n3)["evidence_digest"] != before)
    mark = sl.mark()
    submit_password(PASSWORD)
    ready(DIALOG + " #choose-heading")
    text = dialog_text()
    check("The state changed since you reviewed it" in text, "stale notice missing", text=text[:600])
    check(not finds(DIALOG + " #resolution-password") and not finds(DIALOG + " #confirm-heading"), "confirmation not cleared")
    check(not any(r.is_selected() for r in finds(DIALOG + " input[name=declared]")), "stale choice kept although no longer allowed")
    new = rg.fresh_view(n3)
    check("yes" in find(DIALOG + " article.resolution-target").text, "refreshed view does not show the container running")
    check(not find(DIALOG + " input[value=not_done]").is_enabled() and find(DIALOG + " input[value=verified_done]").is_enabled(), "allowed outcomes not refreshed", allowed=new["allowed"])
    check(rg.core_job(n3["job"]) == "needs_intervention", "stale submit changed state")
    check(rg.lifecycle_events(mark) == [], "stale submit caused Engine events")
    check(active()["id"] == "choose-heading", "focus not moved to the refreshed decision heading", active=active())
    shot("12-stale-evidence-refreshed", "#choose-heading")
    secret_scan("stale")
    statuses = [r["status"] for r in evidence["network"] if r["method"] == "POST" and r["url"].endswith("/resolution")]
    check(statuses[-1] == 409, "stale POST status", statuses=statuses)
    # decide again on the current evidence
    resolve_via_ui(n3, "verified_done", "Container now runs after the manual fix")
    return {"post_statuses": statuses[-2:]}


@case("9 successful resolve: record shown, banner entry gone, jobs list refreshed, no password left, focus recoverable")
def success():
    home(ADMIN)
    h = rg.jobs["H"]
    n_before = attention_count("section[aria-labelledby='operations-heading']")
    label = open_dialog(h)
    resolve_via_ui(h, "verified_done", "Container restarted at the expected time")
    text = dialog_text()
    check("Resolved" in text and "Intended end state is present" in text and "Container restarted at the expected time" in text, "resolved record", text=text[:500])
    check("The containers are unlocked" in text, "unlock sentence")
    check(active()["id"] == "resolved-heading", "focus not on the Resolved heading", active=active())
    check(not finds(DIALOG + " #resolution-password"), "password field still present")
    secret_scan("success")
    shot("13-resolved-record", "#resolved-heading")
    # list refreshed behind the dialog
    wait_for(lambda: any("resolved" in r for r in row_state(h)))
    close_dialog()
    n_after = attention_count("section[aria-labelledby='operations-heading']")
    check(n_after == n_before - 1 and not [b for b in finds("section.attention button") if h["ids"][0][:12] in (b.get_attribute("aria-label") or "")], "banner entry not removed", before=n_before, after=n_after)
    where = active()
    shot("14-banner-after-resolve")
    a11y = {"focus_after_close_of_removed_opener": where}
    evidence["a11y"].append({"case": 9, **a11y})
    check(where["tag"] == "h3" and "Needs attention" in where["text"], "focus not on the banner heading after the opener left the page", active=where)
    return {"banner_before": n_before, "banner_after": n_after, "rows": row_state(h), "focus_after_close": where}


@case("10 pending (202): executor stopped, message shown, identical decision repeated safely")
def pending():
    from selenium.webdriver.common.keys import Keys  # noqa: F401

    home(ADMIN)
    n2 = rg.jobs["N2"]
    open_dialog(n2)
    choose("not_done")
    set_reason("Start never reached any member")
    review()
    ready(DIALOG + " #resolution-password")
    # The executor must die only after core has recorded the request (earlier, the POST is refused with 503
    # because core cannot gather evidence): hold dockerd, kill the executor and keep systemd from restarting it.
    stop, seen = threading.Event(), {}
    executor_pid, docker_pid = rg.pid_of(rg.EXEC_UNIT), rg.dockerd_pid()

    def killer():
        connection = sqlite3.connect(fx.CORE_DB, timeout=1)
        while not stop.is_set():
            try:
                found = connection.execute("SELECT 1 FROM job_resolutions WHERE job=?", (n2["job"],)).fetchone()
            except sqlite3.Error:
                continue
            if found:
                os.kill(docker_pid, signal.SIGSTOP)
                os.kill(executor_pid, signal.SIGKILL)
                fx.run("systemctl", "stop", rg.EXEC_UNIT, check=False)
                os.kill(docker_pid, signal.SIGCONT)
                seen["killed"] = True
                return

    thread = threading.Thread(target=killer)
    thread.start()
    try:
        submit_password(PASSWORD)
        ready(DIALOG + " .notice", "Recorded, waiting for the executor", timeout=60)
        text = dialog_text()
        check("Submitting the same decision again is safe" in text and "Already recorded." in text, "pending messages", text=text[:700])
        check(find(DIALOG + " button[type=submit]").text == "Submit the same decision again", "button label")
        check(rg.core_job(n2["job"]) == "needs_intervention" and len(rg.resolution_row(n2["job"])) == 1, "request not recorded", row=rg.resolution_row(n2["job"]))
        check(find(DIALOG + " #resolution-password").get_attribute("value") == "", "password kept after pending")
        shot("15-pending", ".notice")
        secret_scan("pending")
        statuses = [r["status"] for r in evidence["network"] if r["method"] == "POST" and r["url"].endswith("/resolution")]
        check(statuses[-1] == 202, "pending POST status", statuses=statuses)
    finally:
        stop.set()
        thread.join(10)
        try:
            os.kill(docker_pid, signal.SIGCONT)
        except OSError:
            pass
        fx.run("systemctl", "start", rg.EXEC_UNIT)
    check(seen.get("killed"), "never saw the recorded request to kill the executor at", seen=seen)
    fx.wait_container_ready(timeout=45)
    sl.wait_core()
    mark = sl.mark()
    outcome = {}
    submit_password(PASSWORD, label="Submit the same decision again")
    try:
        ready(DIALOG + " #resolved-heading", timeout=45)
        outcome["repeat"] = "resolved"
    except AssertionError:
        outcome["repeat"] = "no resolved heading; dialog: " + dialog_text()[:500]
    drain()
    statuses = [r["status"] for r in evidence["network"] if r["method"] == "POST" and r["url"].endswith("/resolution")]
    outcome["post_statuses"] = statuses[-3:]
    shot("16-pending-then-resolved", "#resolved-heading")
    check(outcome["repeat"] == "resolved", "repeat did not resolve", outcome=outcome)
    wait_for(lambda: rg.core_job(n2["job"]) == "resolved")
    check(rg.job_locks(n2["job"]) == [] and len(rg.resolution_row(n2["job"])) == 1, "locks or duplicate record after repeat")
    check(rg.lifecycle_events(mark) == [], "repeat caused Engine events")
    secret_scan("pending-repeat")
    wait_for(lambda: any("resolved" in r for r in row_state(n2)))
    close_dialog()
    return outcome


@case("11 keyboard-only: open, choose, reason, review, confirm, submit and close with Tab/Space/Enter/Escape")
def keyboard():
    from selenium.webdriver.common.keys import Keys

    home(ADMIN)
    n1 = rg.jobs["N1"]
    label = open_dialog(n1, keyboard=True)
    check(active()["inDialog"], "focus not in dialog after Enter on the opener")
    trail = []
    tab_until(lambda a: a["type"] == "radio" and not trail.append(a) and True, limit=20)
    press(Keys.SPACE)
    check(find(DIALOG + " input[value=not_done]").is_selected(), "Space did not select not_done", active=active())
    press(Keys.TAB)
    check(active()["id"] == "resolution-reason", "Tab from the radio did not reach the reason", active=active())
    type_text("Start refused at the runtime, nothing changed")
    press(Keys.TAB)
    check(active()["text"] == "Review decision", "Review button not next", active=active())
    press(Keys.ENTER)
    ready(DIALOG + " #confirm-heading")
    check(active()["id"] == "confirm-heading", "focus not on the confirmation heading", active=active())
    press(Keys.TAB)
    check(active()["id"] == "resolution-password", "password not next", active=active())
    mark = sl.mark()
    rg.pace()
    type_text(PASSWORD)
    press(Keys.TAB)
    check(active()["text"] == "Resolve job", "Resolve button not next", active=active())
    press(Keys.SPACE)
    ready(DIALOG + " #resolved-heading")
    check(active()["id"] == "resolved-heading", "focus not on Resolved heading", active=active())
    time.sleep(3)
    check(rg.lifecycle_events(mark) == [] and rg.core_job(n1["job"]) == "resolved", "keyboard resolve state")
    secret_scan("keyboard")
    shot("17-keyboard-resolved", "#resolved-heading")
    press(Keys.TAB)
    check(active()["label"] == "Close resolution dialog", "Close not reachable after resolution", active=active())
    press(Keys.SPACE)
    gone(DIALOG)
    # Escape path was verified in case 3; verify here that the page is usable by keyboard afterwards
    where = active()
    evidence["a11y"].append({"case": 11, "focus_after_keyboard_close": where})
    check(where["tag"] == "h2" and "Recent container operations" in where["text"], "focus not on the jobs heading after the last banner entry left", active=where)
    return {"focus_after_close": where}


@case("12 final: remaining job (D3) resolved by divergence in the stack list; no console errors; nothing sensitive anywhere")
def final():
    home(ADMIN)
    d3 = rg.jobs["D3"]
    stacks_view()
    n_before = attention_count("section[aria-labelledby='stack-operations-heading']")
    open_dialog(d3)
    resolve_via_ui(d3, "accepted_divergence", "Checked the host: members differ by hand")
    shot("18-stack-divergence-resolved", "#resolved-heading")
    wait_for(lambda: any("resolved" in r for r in row_state(d3)))
    close_dialog()
    n_after = attention_count("section[aria-labelledby='stack-operations-heading']")
    check(n_after < n_before, "stack banner not reduced", before=n_before, after=n_after)
    secret_scan("final")
    drain()
    severe = [e for e in evidence["console"] if e["level"] == "SEVERE"]
    # Chromium logs every non-2xx fetch as a SEVERE console line. Expected ones: the pre-login session probe (401),
    # the wrong-password POST (401), the stale-evidence POST (409), and the unrelated storage-mounts panel, which
    # this fixture does not configure (503). Anything else is unexpected.
    allowed = (("/api/v1/auth/session", "401"), ("/resolution", "401"), ("/resolution", "409"), ("/api/v1/storage/mounts/targets", "503"),
               ("/api/v1/storage/mounts/", "403"))
    unexpected = [e for e in severe if not any(p in e["message"] and "status of " + c in e["message"] for p, c in allowed)]
    evidence["console_expected_http_errors"] = len(severe) - len(unexpected)
    check(unexpected == [], "unexpected console errors", errors=unexpected[:5])
    check(not [e for e in evidence["console"] if e["level"] == "WARNING" and "react" in e["message"].lower()], "react warnings")
    return {"deliberate_http_errors_logged_by_chromium": len(severe), "console_entries": len(evidence["console"])}


# ---------------------------------------------------------------- driver
def create_operator():
    """An operator-role user (same password as the administrator) with broad grants, for real UI logins."""
    with sqlite3.connect(fx.CORE_DB) as connection:
        hashed = connection.execute("SELECT password_hash FROM users WHERE id=?", (rg.session["principal"]["id"],)).fetchone()[0]
        connection.execute("INSERT INTO users VALUES(?,?,?,?,1)", ("e" * 64, OPERATOR, hashed, '"operator"'))
        for operation in ("health_read", "container_manage", "deployment_manage"):
            connection.execute("INSERT INTO grants VALUES(?,?,?)", ("e" * 64, json.dumps(operation), "*"))


def as_operator(induce, *args):
    """Run an induction through the operator's own API session (jobs belong to the principal that queued them)."""
    rg.pace()
    status, headers, view = fx.http("/api/v1/auth/login", "POST", {"username": OPERATOR, "password": PASSWORD}, origin="https://localhost")
    check(status == 200, "operator login failed", status=status)
    saved = dict(sl.session)
    sl.session.update(cookie=headers["set-cookie"].split(";")[0], csrf=view["csrf_token"], principal=view["principal"])
    try:
        return induce(*args)
    finally:
        sl.session.clear()
        sl.session.update(saved)


def provision():
    rg.fixture["bad_dirs"] = {}
    sl.retire_stack("sl1")
    rg.fixture["slow"] = {"sl2": sl.make_stack("sl2", run=False, slow=True)}
    rg.make_bad_first_stack("sl4")
    rg.make_bad_first_stack("sl5")
    solos = {}
    for n in range(1, 5):
        name = f"sl-solo-{n}"
        solos[name] = rg.make_solo(name, rg.SOLO_ROOT / name)
        rg.fixture["bad_dirs"][name] = rg.SOLO_ROOT / name
    rg.fixture["solos"] = solos
    members = [i for ids in sl.fixture["stacks"].values() for i in ids]
    sl.fixture["managed"] = sorted({sl.fixture["holder"], *members, *solos.values()})
    sl.write_policy(managed_containers=sl.fixture["managed"])
    for identifier in sl.fixture["stacks"]["sl2"]:
        fx.docker("start", identifier)
    fx.docker("start", sl.fixture["holder"])


def install_ui_override():
    """Replace the packaged UI by a dist built from the commit under test (the package copies dist verbatim)."""
    archive = Path("/root/ui-dist.tar.gz")
    if not archive.exists():
        return None
    root = Path("/usr/lib/limeos/ui")
    before = {str(p.relative_to(root)): hashlib.sha256(p.read_bytes()).hexdigest() for p in sorted(root.rglob("*")) if p.is_file()}
    fx.run("rm", "-rf", str(root))
    root.mkdir(mode=0o755)
    fx.run("tar", "-xzf", str(archive), "-C", str(root))
    fx.run("chmod", "-R", "a+rX", str(root))
    return before


def start_browser():
    fx.run("apt-get", "install", "-y", "--no-install-recommends", "caddy", "chromium", "chromium-driver", "python3-selenium", timeout=300)
    fx.run("install", "-m", "0644", "/usr/share/doc/limeos/Caddyfile.example", "/etc/caddy/Caddyfile")
    fx.run("systemctl", "restart", "caddy")
    from selenium import webdriver
    from selenium.webdriver.chrome.service import Service

    options = webdriver.ChromeOptions()
    options.binary_location = "/usr/bin/chromium"
    for argument in ("--headless=new", "--no-sandbox", "--disable-dev-shm-usage", "--disable-background-networking",
                     "--window-size=1100,1500", "--user-data-dir=/root/res-browser-profile"):
        options.add_argument(argument)
    options.accept_insecure_certs = True
    options.set_capability("goog:loggingPrefs", {"performance": "ALL", "browser": "ALL"})
    driver = webdriver.Chrome(service=Service("/usr/bin/chromedriver"), options=options)
    driver.set_page_load_timeout(30)
    driver.set_script_timeout(15)
    return driver


def main(package_version="0.4.17+resbrowser.1"):
    global driver
    if os.getuid() != 0 or socket.gethostname() != "limeos-p01-test":
        raise SystemExit("Disposable guest required")
    repository, output = map(Path, sys.argv[1:])
    rg.session["version"] = package_version
    started = time.time()
    document = {"summary": "fail", "package_version": package_version}
    try:
        engine_id = sl.setup(repository, package_version)
        provision()
        sl.wait_core()
        rg.induce_container_restart("H")
        rg.induce_stack_restart("D3", "sl2", 1)
        rg.induce_bad_start_container("N1", "sl-solo-1")
        rg.induce_bad_start_container("N3", "sl-solo-2")
        rg.induce_bad_start_stack("N2", "sl4")
        create_operator()
        as_operator(rg.induce_bad_start_container, "N5", "sl-solo-3")  # queued by the operator: only the operator lists it
        as_operator(rg.induce_bad_start_stack, "N6", "sl5")
        evidence["induced"] = {k: {"job": v["job"], "kind": v["kind"], "operation": v["operation"], "targets": len(v["ids"])} for k, v in rg.jobs.items()}
        evidence["packaged_ui_sha256"] = install_ui_override()
        driver = start_browser()
        evidence["browser_version"] = driver.capabilities["browserVersion"]
        evidence["ui_sha256"] = {str(p.relative_to("/usr/lib/limeos/ui")): hashlib.sha256(p.read_bytes()).hexdigest()
                                 for p in sorted(Path("/usr/lib/limeos/ui").rglob("*")) if p.is_file()}
        for run in order:
            run()
        document.update(engine_id=engine_id, setup_seconds=round(time.time() - started, 1), facts={"final_locks": sl.locks()})
    finally:
        if driver is not None:
            try:
                drain()
                evidence["network_summary"] = [{k: r[k] for k in ("method", "url", "status")} for r in evidence["network"] if r["url"].startswith("/api")][-400:]
                evidence["network"] = []
                driver.quit()
            except Exception:  # noqa: BLE001
                pass
        document.update(cases=results, summary="pass" if results and all(r["status"] == "pass" for r in results.values()) else "fail", evidence=evidence)
        encoded = json.dumps(document, indent=2, default=str) + "\n"
        output.write_text(encoded)
        brief = {k: (v["status"], v.get("error", "")[:300]) for k, v in results.items()}
        print("RESOLUTION_BROWSER_RESULT " + json.dumps(brief), flush=True)
    if document["summary"] != "pass":
        raise SystemExit(1)


if __name__ == "__main__":
    main()
