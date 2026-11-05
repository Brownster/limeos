"""Actual packaged B1 browser flow, exclusively inside the disposable Debian VM."""

import base64
import hashlib
import json
import os
import secrets
import signal
import sqlite3
import sys
import tarfile
import time
from contextlib import closing
from pathlib import Path
from urllib.parse import urlsplit

import p04_automation_guest as fixture
from current_authority_schema import current_authority_schema
from guest import executor_probe_code
from p04_fixture_readiness import wait_storage_reader

run = fixture.run
require = fixture.require
ORIGIN = "https://localhost"


class NetworkEvidence:
    """Retain only local request method/path/status, never browser log payloads."""

    def __init__(self):
        self.requests = {}
        self.responses = []

    def drain(self, driver):
        for entry in driver.get_log("performance"):
            message = json.loads(entry["message"])["message"]
            event, fields = message["method"], message.get("params", {})
            if event == "Network.requestWillBeSent":
                request = fields["request"]
                url = urlsplit(request["url"])
                if (url.scheme, url.netloc) == ("https", "localhost"):
                    self.requests[fields["requestId"]] = {
                        "method": request["method"],
                        "path": url.path,
                    }
            elif event == "Network.responseReceived":
                request = self.requests.pop(fields["requestId"], None)
                if request is not None:
                    self.responses.append(
                        {**request, "status": fields["response"]["status"]}
                    )
            require(
                len(self.requests) + len(self.responses) <= 2048,
                "browser network evidence exceeds bound",
            )

    def statuses(self, path):
        return [
            row["status"]
            for row in self.responses
            if row["path"] == path and row["method"] == "POST"
        ]


def wait_for(observe, timeout=20):
    """Poll reads under one deadline; mutations never enter this helper."""
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        value = observe()
        if value:
            return value
        time.sleep(min(0.2, max(0, deadline - time.monotonic())))
    raise AssertionError("Installed browser observation deadline expired")


def oom_events(path=Path("/sys/fs/cgroup/memory.events")):
    return {
        key: int(value)
        for line in path.read_text().splitlines()
        for key, value in [line.split()]
        if key in ("oom", "oom_kill", "oom_group_kill")
    }


def screenshot(driver, evidence, name):
    require(
        all(
            not field.get_attribute("value")
            for field in driver.find_elements("css selector", 'input[type="password"]')
        ),
        "password field must be empty before screenshot",
    )
    data = driver.get_screenshot_as_png()
    require(len(data) <= 512 * 1024, "screenshot exceeds bound")
    evidence.setdefault("screenshots", {})[name] = {
        "sha256": hashlib.sha256(data).hexdigest(),
        "png_base64": base64.b64encode(data).decode("ascii"),
    }


def restart_core():
    before = int(fixture.unit("limeos-core")["MainPID"])
    require(before > 1, "Core is not live")
    os.kill(before, signal.SIGKILL)

    def restarted():
        after = fixture.unit("limeos-core")
        if int(after["MainPID"]) <= 1 or int(after["MainPID"]) == before:
            return None
        try:
            status = fixture.api.http("/api/v1/auth/session")[0]
        except OSError:
            return None
        return {"before_pid": before, "after": after} if status == 401 else None

    return wait_for(restarted)


def browser_flow(driver, password, identifier, evidence):
    from selenium.common.exceptions import (
        NoSuchElementException,
        StaleElementReferenceException,
    )

    network = NetworkEvidence()
    evidence["http"] = network.responses

    def element(selector):
        return driver.find_element("css selector", selector)

    def ready(selector, text=None):
        def observed():
            try:
                value = element(selector)
                return (
                    value
                    if value.is_displayed()
                    and value.is_enabled()
                    and (text is None or text in value.text)
                    else None
                )
            except (NoSuchElementException, StaleElementReferenceException):
                return None

        return wait_for(observed)

    def button(label):
        # Accessible visible labels, not implementation callbacks or JS injection.
        def observed():
            matches = [
                value
                for value in driver.find_elements("tag name", "button")
                if value.text.replace("↗", "").strip() == label
                and value.is_displayed()
                and value.is_enabled()
            ]
            require(len(matches) <= 1, "ambiguous visible browser control: " + label)
            return matches[0] if matches else None

        return wait_for(observed)

    def login():
        ready("#username").send_keys("b1-browser-admin")
        element("#password").send_keys(password)
        button("Sign in").click()
        ready("h1", "At home.")

    def fill(selector, value):
        field = ready(selector)
        field.clear()
        field.send_keys(value)

    def history_notice(event, generation=None):
        deadline = time.monotonic() + 95
        while time.monotonic() < deadline:
            button("Refresh alert history").click()
            ready("#health-history-panel .notifications-table")
            network.drain(driver)
            value = fixture.notification(
                fixture.history(), "container:" + identifier, event, generation
            )
            label = "Incident opened" if event == "incident" else "Recovered"
            rows = driver.find_elements("css selector", ".notifications-table tbody tr")
            if value and any(
                label in row.text and identifier in row.text and "Delivered" in row.text
                for row in rows
            ):
                return value
            time.sleep(min(1, max(0, deadline - time.monotonic())))
        raise AssertionError("Real timer notification did not reach installed browser")

    evidence["stage"] = "browser_authentication"
    driver.get(ORIGIN + "/")
    login()
    button("Manage health schedules").click()
    fill("#observation-interval", "1")
    original = fixture.history()
    button("Create observation schedule").click()
    ready("#health-schedules-panel .error")
    network.drain(driver)
    require(
        network.statuses("/api/v1/automation/schedules") == [403],
        "actual create did not receive permission refusal",
    )
    require(fixture.history() == original, "refused UI create changed durable history")
    element("#scheduling-password").send_keys(password)
    button("Enable health scheduling").click()
    ready("h1", "Welcome home.")
    login()
    evidence["passed"].append(
        "actual browser login, refused create, password enablement and reauthentication"
    )

    evidence["stage"] = "browser_policy_and_schedule"
    button("Manage alert policy").click()
    fill("#health-fail-threshold", "1")
    button("Save health settings").click()
    ready("#health-policy-panel", "Health settings saved.")
    button("Manage health schedules").click()
    fill("#observation-interval", "1")
    button("Create observation schedule").click()
    ready(".schedules-table tbody tr abbr")
    created = fixture.history()
    require(len(created["schedules"]) == 1, "UI did not create one schedule")
    schedule_id = created["schedules"][0][0]
    evidence["schedule_id"] = schedule_id
    button("View alerts and notifications").click()
    ready("#health-history-panel .notifications-table")
    evidence["stage"] = "browser_incident"
    opening = history_notice("incident")
    evidence["opening"] = opening
    original = fixture.history()
    fixture.assert_identities(original)
    require(len(original["jobs"]) == 1, "incident did not settle exactly once")
    evidence["before_restart"] = original
    screenshot(driver, evidence, "incident-desktop")
    evidence["restarts"] = [restart_core()]
    stable_until = time.monotonic() + 8
    while time.monotonic() < stable_until:
        require(fixture.history() == original, "restart changed original identities")
        time.sleep(0.25)
    # Refresh through the actual session; no cookie injection or API substitution.
    driver.refresh()
    ready("h1", "At home.")
    button("View alerts and notifications").click()
    ready("#health-history-panel", "Incident opened")
    evidence["after_restart"] = fixture.history()
    require(
        evidence["after_restart"] == original, "browser restart refresh changed history"
    )
    evidence["passed"].append(
        "UI-created real timer incident is delivered once and survives actual Core crash with the same session"
    )

    evidence["stage"] = "browser_recovery"
    run("docker", "start", identifier)
    require(
        json.loads(run("docker", "inspect", identifier).stdout)[0]["State"]["Running"],
        "independent Engine recovery check failed",
    )
    evidence["recovery"] = history_notice("recovery", opening["event"]["generation"])
    current = fixture.history()
    fixture.assert_identities(current)
    require(
        len(current["jobs"]) == 2 and len(current["notifications"]) == 2,
        "recovery did not settle exactly once",
    )
    ready("#health-history-panel", "No open incident")
    screenshot(driver, evidence, "recovery-desktop")
    element(
        f'button[aria-label="Acknowledge incident for container:{identifier}"]'
    ).click()
    ready(".notifications-table", "Acknowledged")
    button("Manage health schedules").click()
    ready(".schedules-table tbody tr abbr")
    element(f'button[aria-label="Pause {schedule_id}"]').click()
    ready(".schedules-table", "Paused")
    before_restart = fixture.history()
    require(before_restart["schedules"][0][1:3] == [2, 1], "UI pause revision differs")
    evidence["restarts"].append(restart_core())
    driver.refresh()
    ready("h1", "At home.")
    button("Manage health schedules").click()
    ready(".schedules-table", "Paused")
    button("View alerts and notifications").click()
    ready(".notifications-table", "Acknowledged")
    require(
        fixture.history() == before_restart,
        "pause/acknowledgment changed across restart",
    )
    network.drain(driver)
    for path, expected in {
        "/api/v1/auth/login": [200, 200],
        "/api/v1/auth/capabilities/observe-health/enable": [200],
        "/api/v1/automation/health-policy": [200],
        "/api/v1/automation/schedules": [403, 201],
        "/api/v1/automation/notifications/" + opening["id"] + "/acknowledgment": [200],
        "/api/v1/automation/schedules/" + schedule_id + "/pause": [200],
    }.items():
        require(
            network.statuses(path) == expected,
            "browser mutation count/status differs: " + path,
        )
    driver.set_window_size(390, 844)
    screenshot(driver, evidence, "paused-mobile")
    evidence["final_history"] = fixture.history()
    evidence["passed"].append(
        "same-container recovery, UI acknowledgment and UI pause persist after a second Core crash"
    )


def main(package_version="0.4.13+coreopen.1"):
    fixture.disposable_guard()
    repo, output = map(Path, sys.argv[1:])
    evidence = {
        "stage": "install",
        "package_version": package_version,
        "passed": [],
        "limitations": [
            "AMD64 installed browser flow; ARM64/Pi, upgrade/return, soak and full B1/P04 remain open",
            "Container-only partial health coverage; no application storage/deployment/external delivery effect",
            "Disposable local certificate accepted and Chromium runs without sandbox inside the throwaway VM; production TLS/browser hardening not qualified",
            "Existing history/policy/save notices incorrectly say automatic observations are unavailable; truthful-status UI acceptance remains open",
        ],
    }
    driver = None
    try:
        evidence["oom_before"] = oom_events()
        run(
            "install",
            "-m",
            "0644",
            str(repo / "limeos-archive-keyring.gpg"),
            "/usr/share/keyrings/limeos-test.gpg",
        )
        Path("/etc/apt/sources.list.d/limeos-test.list").write_text(
            f"deb [signed-by=/usr/share/keyrings/limeos-test.gpg] file:{repo} stable main\n"
        )
        run("apt-get", "update", timeout=120)
        run(
            "apt-get",
            "install",
            "-y",
            "--no-install-recommends",
            "limeos=" + package_version,
            "busybox-static",
            "caddy",
            "chromium",
            "chromium-driver",
            "python3-selenium",
            timeout=240,
        )
        fixture.api.eventually(lambda: Path("/run/limeos-core/core.sock").exists())
        with closing(
            sqlite3.connect(fixture.DB.as_uri() + "?mode=ro", uri=True)
        ) as conn:
            evidence["authority_schema"] = conn.execute(
                "PRAGMA user_version"
            ).fetchone()[0]
        require(
            evidence["authority_schema"] == current_authority_schema(),
            "installed schema differs from trusted source",
        )
        evidence["binary_sha256"] = {
            path.name: fixture.sha(path)
            for path in sorted(Path("/usr/lib/limeos").glob("limeos*"))
            if path.is_file()
        }
        evidence["ui_sha256"] = {
            str(path.relative_to("/usr/lib/limeos/ui")): fixture.sha(path)
            for path in sorted(Path("/usr/lib/limeos/ui").rglob("*"))
            if path.is_file()
        }
        evidence["package_sha256"] = {
            path.name: fixture.sha(path) for path in sorted(repo.rglob("*.deb"))
        }
        evidence["fixture_sha256"] = fixture.sha(__file__)
        config = Path("/etc/limeos/core.json")
        require(
            json.loads(config.read_text())["origin"] == ORIGIN, "package origin differs"
        )
        immutable = [config, Path("/etc/limeos/system-policy/storage.json")]
        for unit in ("limeos-core", "limeos-storage-reader"):
            immutable.append(
                Path(
                    run(
                        "systemctl", "show", unit, "-p", "FragmentPath", "--value"
                    ).stdout.strip()
                )
            )
        evidence["immutable_before"] = {
            str(path): fixture.sha(path) for path in immutable
        }
        evidence["docker"] = fixture.engine_fixture.actual_docker()
        with tarfile.open("/root/b1-browser-rootfs.tar", "w") as archive:
            archive.add("/bin/busybox", arcname="bin/busybox")
        run(
            "docker", "import", "/root/b1-browser-rootfs.tar", "limeos-b1-browser:local"
        )
        identifier = run(
            "docker",
            "create",
            "--name",
            "limeos-b1-browser",
            "--restart",
            "unless-stopped",
            "--network",
            "none",
            "limeos-b1-browser:local",
            "/bin/busybox",
            "sleep",
            "3600",
        ).stdout.strip()
        require(
            len(identifier) == 64 and all(c in "0123456789abcdef" for c in identifier),
            "invalid real container ID",
        )
        require(
            not json.loads(run("docker", "inspect", identifier).stdout)[0]["State"][
                "Running"
            ],
            "fixture container must begin stopped",
        )
        evidence["container_id"] = identifier
        run("systemctl", "start", "limeos-storage-reader")
        evidence["reader_ready"] = wait_storage_reader(run, executor_probe_code())
        password = secrets.token_urlsafe(24)
        ctl = ["/usr/lib/limeos/limeosctl", "--socket", "/run/limeos-core/core.sock"]
        token = json.loads(run(*ctl, "bootstrap").stdout)["token"]
        fixture.api.run(
            *ctl,
            "enroll",
            input=json.dumps(
                {"token": token, "username": "b1-browser-admin", "password": password}
            ),
            timeout=60,
        )
        run(
            "install",
            "-m",
            "0644",
            "/usr/share/doc/limeos/Caddyfile.example",
            "/etc/caddy/Caddyfile",
        )
        run("systemctl", "restart", "caddy")
        evidence["dependencies"] = run(
            "dpkg-query",
            "-W",
            "-f=${Package} ${Version}\n",
            "chromium",
            "chromium-driver",
            "python3-selenium",
            "caddy",
        ).stdout
        from selenium import webdriver
        from selenium.webdriver.chrome.service import Service

        options = webdriver.ChromeOptions()
        options.binary_location = "/usr/bin/chromium"
        for argument in (
            "--headless=new",
            "--no-sandbox",
            "--disable-dev-shm-usage",
            "--disable-background-networking",
            "--window-size=1100,900",
            "--user-data-dir=/root/b1-browser-profile",
        ):
            options.add_argument(argument)
        options.accept_insecure_certs = True
        options.set_capability("goog:loggingPrefs", {"performance": "ALL"})
        driver = webdriver.Chrome(
            service=Service("/usr/bin/chromedriver"), options=options
        )
        driver.set_page_load_timeout(20)
        driver.set_script_timeout(10)
        evidence["browser_version"] = driver.capabilities["browserVersion"]
        browser_flow(driver, password, identifier, evidence)
        evidence["immutable_after"] = {
            str(path): fixture.sha(path) for path in immutable
        }
        require(
            evidence["immutable_after"] == evidence["immutable_before"],
            "browser fixture changed application configuration or units",
        )
        evidence["services"] = {
            name: fixture.service_snapshot(name)
            for name in ("limeos-core", "limeos-storage-reader")
        }
        evidence["oom_after"] = oom_events()
        require(
            evidence["oom_before"] == evidence["oom_after"],
            "guest OOM counter increased",
        )
        evidence.update(stage="complete", summary="pass")
    except Exception as error:
        evidence.update(summary="fail", error_type=type(error).__name__)
        try:
            evidence["history_after_failure"] = fixture.history()
        except (OSError, sqlite3.Error, ValueError) as observer_error:
            evidence["history_after_failure"] = {
                "error_type": type(observer_error).__name__
            }
        # The canonical runner cannot assume it can copy guest files after a
        # failed command. Preserve a separately bounded public packet on stdout.
        public = {key: value for key, value in evidence.items() if key != "screenshots"}
        if len(json.dumps(public).encode()) > 128 * 1024:
            public = {
                key: evidence[key]
                for key in ("stage", "summary", "error_type", "package_version")
            }
        # Exclude exception strings, request bodies, cookies and raw browser logs.
        print(
            "B1_BROWSER_FAILURE " + json.dumps(public, sort_keys=True),
            flush=True,
        )
        raise
    finally:
        try:
            if driver is not None:
                driver.quit()
        except Exception as error:
            evidence.update(summary="fail", browser_cleanup_error=type(error).__name__)
            raise
        finally:
            encoded = json.dumps(evidence, indent=2) + "\n"
            require(
                len(encoded.encode()) <= 2 * 1024 * 1024,
                "browser result exceeds bound",
            )
            output.write_text(encoded)


if __name__ == "__main__":
    main()
