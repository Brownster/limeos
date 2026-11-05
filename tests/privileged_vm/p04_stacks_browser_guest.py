"""Current genuine Stack API/UI layouts in the disposable Debian VM only."""

import json
import secrets
import sqlite3
import sys
import tarfile
from contextlib import closing
from pathlib import Path

import p04_automation_browser_guest as browser
import p04_automation_guest as fixture
from current_authority_schema import current_authority_schema
from p04_reader_resources import ReaderResources

ROOT = Path("/opt/limeos-b2-fixture")
CATALOG = Path("/etc/limeos/management-catalog.json")
API = "/api/v1/deployment/stacks"
run = fixture.api.run
require = fixture.require

# stack index, service, one-off, running. Sixteen complete Engine members.
LAYOUT = (
    (1, "web", False, False),
    (1, "web", False, False),
    (2, "web", False, True),
    (2, "web", False, False),
    (2, "web", True, True),
    (3, "web", False, True),
    (3, "web", False, True),
    (4, "web", True, True),
    (4, "web", True, False),
    (5, "web", False, True),
    (5, "worker", False, True),
    (6, "web", False, False),
    (6, "rogue", False, False),
    (7, "web", False, True),
    (7, "web", False, False),
    (None, None, False, False),
)


def catalog(engine_id, *, maximum=False):
    stacks = []
    for index in range(8):
        directory = ROOT / ("Stack" + str(index))
        services = ["web", "worker"] if index == 5 else ["web"]
        if maximum:
            services = ["web", "worker", *["missing" + str(n) for n in range(14)]]
        stacks.append(
            {
                "stack": "fixture" + str(index),
                "project": "fixture" + str(index),
                "working_directory": str(directory),
                "operator": {
                    "name": "compose.yaml",
                    "sha256": fixture.sha(directory / "compose.yaml"),
                },
                "managed": {"state": "absent"},
                "services": services,
            }
        )
    return {"version": 1, "engine_id": engine_id, "stacks": stacks}


def save_catalog(value):
    data = json.dumps(value).encode()
    require(len(data) <= 16 * 1024, "fixture catalog exceeds original bound")
    temporary = CATALOG.with_suffix(".new")
    with temporary.open("xb") as stream:
        stream.write(data)
    temporary.chmod(0o644)
    temporary.replace(CATALOG)


def counts(rows):
    return {
        "ordinary": sum(not row["one_off"] for row in rows),
        "running": sum(row["running"] and not row["one_off"] for row in rows),
        "one_offs": sum(row["one_off"] for row in rows),
        "one_off_running": sum(row["running"] and row["one_off"] for row in rows),
    }


def check_report(value, declared, members):
    require(
        value["catalog"]["state"] == value["engine"]["state"] == "fresh"
        and value["engine_matches_catalog"] is True,
        "Stack sources are not current/matched",
    )
    require(
        len(value["stacks"]) == 8 and value["other_projects"] == [],
        "Stack declaration cardinality differs",
    )
    require(
        value["standalone_containers"] == sum(row["stack"] is None for row in members),
        "standalone membership differs",
    )
    by_id = {row["stack"]: row for row in value["stacks"]}
    require(
        set(by_id) == {row["stack"] for row in declared["stacks"]},
        "Stack identities differ",
    )
    for declaration in declared["stacks"]:
        key = declaration["stack"]
        row = by_id[key]
        expected = [member for member in members if member["stack"] == key]
        require(
            row["working_directory"] == declaration["working_directory"]
            and row["project"] == declaration["project"],
            "literal Stack identity differs",
        )
        require(
            row["config_files"] == [declaration["working_directory"] + "/compose.yaml"]
            and row["files"] == {"state": "not_inspected"},
            "unimplemented source inspection was misrepresented",
        )
        require(
            row["counts"] == counts(expected),
            "Stack totals differ from independent Engine membership",
        )
        ordinary_services = {
            member["service"] for member in expected if not member["one_off"]
        }
        missing = len(set(declaration["services"]) - ordinary_services)
        require(row["missing_services"] == missing, "missing ordinary services differ")
        extra = {member["service"] for member in expected} - set(
            declaration["services"]
        )
        require(
            row["issues"] == (["undeclared_services"] if extra else []),
            "Stack issue projection differs",
        )
        expected_names = set(declaration["services"]) | {
            member["service"] for member in expected
        }
        require(
            {service["name"] for service in row["services"]} == expected_names,
            "service projection omitted members/declarations",
        )
        for service in row["services"]:
            expected_rows = [
                member for member in expected if member["service"] == service["name"]
            ]
            require(
                service["counts"] == counts(expected_rows)
                and service["declared"] == (service["name"] in declaration["services"]),
                "service counts/registration differ",
            )
            actual = {
                item["container"]["resource"]: (
                    item["container"]["running"],
                    item["one_off"],
                )
                for item in service["members"]
            }
            require(
                actual
                == {
                    "container:" + item["id"]: (item["running"], item["one_off"])
                    for item in expected_rows
                },
                "full-ID service member binding differs",
            )
        total = counts(expected)
        runtime = (
            "no_containers"
            if total["ordinary"] == 0
            else "stopped"
            if total["running"] == 0
            else "running"
            if total["running"] == total["ordinary"] and not missing and not extra
            else "partial"
        )
        require(row["runtime"] == runtime, "Stack runtime status differs")


def authority_counts():
    with closing(
        sqlite3.connect(fixture.DB.as_uri() + "?mode=ro", uri=True, timeout=2)
    ) as conn:
        conn.execute("PRAGMA query_only=ON").close()
        return {
            table: conn.execute("SELECT count(*) FROM " + table).fetchone()[0]
            for table in (
                "jobs",
                "approvals",
                "automation_schedules",
                "automation_occurrences",
                "automation_notifications",
            )
        }


class ApiReader:
    def __init__(self, password):
        status, headers, _ = fixture.api.http(
            "/api/v1/auth/login",
            "POST",
            {"username": "b2-reader", "password": password},
        )
        require(status == 200, "independent actual API login failed")
        self.cookie = headers["set-cookie"].split(";", 1)[0]
        self.last = None

    def read(self):
        status, _, value = fixture.api.http(API, cookie=self.cookie)
        require(status == 200, "Stack API read refused")
        self.last = value
        return value

    def fresh(self, declared, members, *, newer=None):
        def current():
            value = self.read()
            if (
                value["catalog"]["state"] != "fresh"
                or value["engine"]["state"] != "fresh"
            ):
                return None
            if newer is not None and value["catalog"]["sampled_at"] <= newer:
                return None
            try:
                check_report(value, declared, members)
            except ValueError:
                return None  # wait for a genuinely new background sample
            return value

        return browser.wait_for(current, timeout=75)


def create_members():
    with tarfile.open(ROOT / "rootfs.tar", "w") as archive:
        archive.add("/bin/busybox", arcname="bin/busybox")
    run("docker", "import", str(ROOT / "rootfs.tar"), "limeos-b2:local")
    result = []
    for number, (index, service, one_off, running) in enumerate(LAYOUT):
        arguments = []
        if index is not None:
            directory = str(ROOT / ("Stack" + str(index)))
            for label in (
                "com.docker.compose.project=fixture" + str(index),
                "com.docker.compose.service=" + service,
                "com.docker.compose.project.working_dir=" + directory,
                "com.docker.compose.project.config_files="
                + directory
                + "/compose.yaml",
                "com.docker.compose.oneoff=" + ("True" if one_off else "False"),
            ):
                arguments.extend(["--label", label])
        for name in ("BindA", "BindB"):
            arguments.extend(
                ["--mount", f"type=bind,src={ROOT / name},dst=/data/{name},readonly"]
            )
        identifier = run(
            "docker",
            "create",
            "--name",
            "limeos-b2-" + str(number),
            "--network=none",
            *arguments,
            "limeos-b2:local",
            "/bin/busybox",
            "sleep",
            "3600",
        ).stdout.strip()
        require(
            len(identifier) == 64 and all(c in "0123456789abcdef" for c in identifier),
            "invalid Engine member ID",
        )
        if running:
            run("docker", "start", identifier)
        result.append(
            {
                "id": identifier,
                "stack": "fixture" + str(index) if index is not None else None,
                "service": service,
                "one_off": one_off,
                "running": running,
            }
        )
    independent = json.loads(
        run("docker", "inspect", *[row["id"] for row in result]).stdout
    )
    require(
        len(independent) == 16
        and sum(row["State"]["Running"] for row in independent) == 8
        and sum(len(row["Mounts"]) for row in independent) == 32,
        "Engine maximum layout differs",
    )
    require(
        {row["Id"]: row["State"]["Running"] for row in independent}
        == {row["id"]: row["running"] for row in result},
        "independent Engine running states differ",
    )
    return result


def browser_controls(driver):
    from selenium.common.exceptions import (
        NoSuchElementException,
        StaleElementReferenceException,
    )

    def ready(selector, text=None):
        def observed():
            try:
                value = driver.find_element("css selector", selector)
                return (
                    value
                    if value.is_displayed() and (text is None or text in value.text)
                    else None
                )
            except (NoSuchElementException, StaleElementReferenceException):
                return None

        return browser.wait_for(observed)

    def click(label):
        def observed():
            matches = [
                value
                for value in driver.find_elements("tag name", "button")
                if value.text.replace("↗", "").strip() == label
                and value.is_displayed()
                and value.is_enabled()
            ]
            require(len(matches) <= 1, "ambiguous Stack UI control")
            return matches[0] if matches else None

        browser.wait_for(observed).click()

    return ready, click


def main(package_version="0.4.14+healthui.1"):
    fixture.disposable_guard()
    repository, output = map(Path, sys.argv[1:])
    evidence = {
        "stage": "install",
        "package_version": package_version,
        "passed": [],
        "states": {},
        "limitations": [
            "AMD64 current installed Stack read UI/layout scenario; not full B2/P04/ARM64/Pi/cutover acceptance",
            "Compose source inspection, review binding and application effects remain unimplemented",
            "FD observations are samples; worker/guardian death, cancellation/deadline and grant replacement matrices remain open",
            "Disposable local certificate accepted/browser sandbox disabled; production TLS/browser hardening not qualified",
        ],
    }
    driver = observer = reader = None
    try:
        evidence["oom_before"] = browser.oom_events()
        run(
            "install",
            "-m",
            "0644",
            str(repository / "limeos-archive-keyring.gpg"),
            "/usr/share/keyrings/limeos-test.gpg",
        )
        Path("/etc/apt/sources.list.d/limeos-test.list").write_text(
            f"deb [signed-by=/usr/share/keyrings/limeos-test.gpg] file:{repository} stable main\n"
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
        evidence["binaries"] = {
            name: fixture.sha(Path("/usr/lib/limeos") / name)
            for name in (
                "limeos-core",
                "limeos-executor",
                "limeos-stack-read-worker",
                "limeos-password-worker",
                "limeosctl",
            )
        }
        evidence["ui"] = {
            str(path.relative_to("/usr/lib/limeos/ui")): fixture.sha(path)
            for path in sorted(Path("/usr/lib/limeos/ui").rglob("*"))
            if path.is_file()
        }
        evidence["packages"] = {
            path.name: fixture.sha(path) for path in sorted(repository.rglob("*.deb"))
        }
        evidence["fixture_sha256"] = fixture.sha(__file__)
        with closing(
            sqlite3.connect(fixture.DB.as_uri() + "?mode=ro", uri=True)
        ) as conn:
            evidence["authority_schema"] = conn.execute(
                "PRAGMA user_version"
            ).fetchone()[0]
        require(
            evidence["authority_schema"] == current_authority_schema(),
            "installed schema differs",
        )
        policy = Path("/etc/limeos/system-policy/storage.json")
        require(
            json.loads(policy.read_text())["allow_stack_read"]
            and run(
                "systemctl", "is-active", "limeos-storage-reader", check=False
            ).returncode
            != 0,
            "reader ceiling/dormancy differs",
        )
        immutable = [policy, Path("/etc/limeos/core.json")]
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
        evidence["engine"] = fixture.engine_fixture.actual_docker()
        for index in range(8):
            directory = ROOT / ("Stack" + str(index))
            directory.mkdir(parents=True, mode=0o755)
            (directory / "compose.yaml").write_text(
                "# Retain fixture source bytes\nservices:\n  web:\n    image: fixture\n"
            )
            (directory / "compose.yaml").chmod(0o644)
        for name in ("BindA", "BindB"):
            (ROOT / name).mkdir(mode=0o755)
            (ROOT / name / "sentinel").write_text(
                "disposable read-only bind sentinel\n"
            )
        sources = [path for path in ROOT.rglob("*") if path.is_file()]
        evidence["source_before"] = {
            str(path): {
                "sha256": fixture.sha(path),
                "uid": path.stat().st_uid,
                "gid": path.stat().st_gid,
                "mode": path.stat().st_mode & 0o7777,
            }
            for path in sources
        }
        declaration = catalog(evidence["engine"]["id"])
        save_catalog(declaration)
        password = secrets.token_urlsafe(24)
        ctl = ["/usr/lib/limeos/limeosctl", "--socket", "/run/limeos-core/core.sock"]
        token = json.loads(run(*ctl, "bootstrap").stdout)["token"]
        run(
            *ctl,
            "enroll",
            input=json.dumps(
                {"token": token, "username": "b2-reader", "password": password}
            ),
            timeout=60,
        )
        reader = ApiReader(password)
        require(
            fixture.api.http(API)[0] == 401
            and fixture.api.http(API, "POST", {}, reader.cookie)[0] == 405,
            "Stack auth/method refusal differs",
        )
        observer = ReaderResources(evidence["binaries"])
        observer.start()
        run("systemctl", "start", "limeos-storage-reader")
        evidence["states"]["empty"] = reader.fresh(declaration, [])
        run(
            "install",
            "-m",
            "0644",
            "/usr/share/doc/limeos/Caddyfile.example",
            "/etc/caddy/Caddyfile",
        )
        run("systemctl", "restart", "caddy")
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
            "--user-data-dir=/root/b2-browser-profile",
        ):
            options.add_argument(argument)
        options.accept_insecure_certs = True
        options.set_capability("goog:loggingPrefs", {"performance": "ALL"})
        driver = webdriver.Chrome(
            service=Service("/usr/bin/chromedriver"), options=options
        )
        evidence["browser_version"] = driver.capabilities["browserVersion"]
        evidence["dependencies"] = run(
            "dpkg-query",
            "-W",
            "-f=${Package} ${Version}\n",
            "chromium",
            "chromium-driver",
            "python3-selenium",
            "caddy",
        ).stdout
        driver.set_page_load_timeout(20)
        driver.set_script_timeout(10)
        ready, click = browser_controls(driver)
        network = browser.NetworkEvidence()
        evidence["http"] = network.responses

        def login():
            ready("#username").send_keys("b2-reader")
            driver.find_element("css selector", "#password").send_keys(password)
            click("Sign in")
            ready("h1", "At home.")

        def stack(index):
            return ready('article[aria-label="Stack fixture' + str(index) + '"]')

        def refreshed(text):
            def observed():
                click("Refresh stacks")
                network.drain(driver)
                panel = ready('section[aria-labelledby="stack-management-heading"]')
                return panel if text in panel.text else None

            return browser.wait_for(observed, timeout=75)

        evidence["stage"] = "empty_browser"
        driver.get("https://localhost/")
        login()
        click("Stacks")
        refreshed("No service containers")
        require(
            len(
                driver.find_elements(
                    "css selector", "article[aria-label^='Stack fixture']"
                )
            )
            == 8,
            "UI omitted registered empty stacks",
        )
        browser.screenshot(
            driver, evidence, "empty-desktop", 'article[aria-label="Stack fixture0"]'
        )
        evidence["passed"].append(
            "all eight registered empty stacks reach actual API and authenticated UI"
        )
        evidence["stage"] = "mixed_layout"
        members = create_members()
        evidence["members"] = members
        mixed = reader.fresh(declaration, members)
        evidence["states"]["mixed"] = mixed
        refreshed("2 of 2 service containers running")
        require(
            "Stopped" in stack(1).text
            and "Partial" in stack(2).text
            and "Running" in stack(3).text,
            "UI representative runtime distinctions differ",
        )
        stack(2).find_element("tag name", "summary").click()
        require(
            "One-off" in stack(2).text and "Service / replica" in stack(2).text,
            "UI member roles absent",
        )
        browser.screenshot(
            driver, evidence, "mixed-desktop", 'article[aria-label="Stack fixture2"]'
        )
        authority = authority_counts()
        before_reads = reader.read()
        timestamps = {
            key: before_reads[key]["sampled_at"] for key in ("catalog", "engine")
        }
        for _ in range(20):
            value = reader.read()
            check_report(value, declaration, members)
            require(
                {key: value[key]["sampled_at"] for key in timestamps} == timestamps,
                "GET burst renewed source timestamps",
            )
        require(
            authority_counts() == authority
            and all(value == 0 for value in authority.values()),
            "inventory reads created durable effect/automation state",
        )
        evidence["passed"].append(
            "sixteen real members/eight running/thirty-two binds retain complete replica/one-off/stopped/standalone distinctions"
        )
        evidence["stage"] = "maximum_catalog"
        maximum = catalog(evidence["engine"]["id"], maximum=True)
        save_catalog(maximum)
        evidence["states"]["maximum"] = reader.fresh(
            maximum, members, newer=mixed["catalog"]["sampled_at"]
        )
        refreshed("15 declared services have no ordinary container")
        stack(7).find_element("tag name", "summary").click()
        require(
            "Services and members · 16" in stack(7).text
            and "17" in stack(6).find_element("tag name", "summary").text,
            "UI maximum declared/undeclared services were truncated",
        )
        browser.screenshot(
            driver, evidence, "maximum-desktop", 'article[aria-label="Stack fixture7"]'
        )
        evidence["stage"] = "catalog_refusal"
        previous = evidence["states"]["maximum"]
        CATALOG.chmod(0o666)

        def refused():
            value = reader.read()
            return (
                value
                if value["catalog"]["state"] == "stale"
                and value["engine"]["state"] == "fresh"
                else None
            )

        refused_value = browser.wait_for(refused, timeout=75)
        require(
            all(
                row["counts"] is None and row["runtime"] == "unknown"
                for row in refused_value["stacks"]
            ),
            "unsafe source retained current counts",
        )
        require(
            refused_value["catalog"]["sampled_at"] == previous["catalog"]["sampled_at"],
            "refused source renewed its sample",
        )
        evidence["states"]["refused"] = refused_value
        refreshed("Current runtime counts are unavailable")
        require(
            "Current container counts are unknown" in stack(7).text,
            "UI presents stale counts as current",
        )
        browser.screenshot(
            driver, evidence, "refused-desktop", 'article[aria-label="Stack fixture7"]'
        )
        CATALOG.chmod(0o644)
        evidence["states"]["recovered"] = reader.fresh(
            maximum, members, newer=previous["catalog"]["sampled_at"]
        )
        refreshed("15 declared services have no ordinary container")
        evidence["passed"].append(
            "maximum 8x16 catalog and genuine unsafe-source refusal/recovery preserve unknown counts and original timestamps"
        )
        evidence["stage"] = "restarts"
        evidence["core_restart"] = browser.restart_core()
        driver.refresh()
        ready("h1", "At home.")
        click("Stacks")
        refreshed("15 declared services have no ordinary container")
        evidence["states"]["core_restarted"] = reader.fresh(maximum, members)
        old_pid = fixture.unit("limeos-storage-reader")["MainPID"]
        old_sample = evidence["states"]["core_restarted"]["catalog"]["sampled_at"]
        run("systemctl", "restart", "limeos-storage-reader")
        new_pid = fixture.unit("limeos-storage-reader")["MainPID"]
        require(old_pid != new_pid and int(new_pid) > 1, "reader did not restart")
        evidence["reader_restart"] = {"before": old_pid, "after": new_pid}
        evidence["states"]["reader_restarted"] = reader.fresh(
            maximum, members, newer=old_sample
        )
        refreshed("15 declared services have no ordinary container")
        click("Sign out")
        ready("h1", "Welcome home.")
        require(
            not driver.find_elements(
                "css selector", "article[aria-label^='Stack fixture']"
            ),
            "abandoned owner facts remain after logout",
        )
        login()
        click("Stacks")
        refreshed("15 declared services have no ordinary container")
        stack(7).find_element("tag name", "summary").click()
        driver.set_window_size(390, 844)
        browser.screenshot(
            driver, evidence, "maximum-mobile", 'article[aria-label="Stack fixture7"]'
        )
        network.drain(driver)
        evidence["passed"].append(
            "Core crash, reader restart and real UI sign-out/relogin recover original Stack identities"
        )
        evidence["resources"] = observer.finish()
        evidence["source_after"] = {
            str(path): {
                "sha256": fixture.sha(path),
                "uid": path.stat().st_uid,
                "gid": path.stat().st_gid,
                "mode": path.stat().st_mode & 0o7777,
            }
            for path in sources
        }
        evidence["immutable_after"] = {
            str(path): fixture.sha(path) for path in immutable
        }
        require(
            evidence["source_before"] == evidence["source_after"]
            and evidence["immutable_before"] == evidence["immutable_after"],
            "read scenario changed source/application bytes or identity",
        )
        require(
            CATALOG.stat().st_mode & 0o7777 == 0o644
            and json.loads(CATALOG.read_text()) == maximum,
            "fixture catalog was not restored",
        )
        evidence["authority_final"] = authority_counts()
        require(
            all(value == 0 for value in evidence["authority_final"].values()),
            "Stack scenario created effect state",
        )
        evidence["oom_after"] = browser.oom_events()
        require(
            evidence["oom_before"] == evidence["oom_after"],
            "guest OOM counter increased",
        )
        evidence.update(summary="pass", stage="complete")
    except Exception as error:
        evidence.update(summary="fail", error_type=type(error).__name__)
        if reader is not None:
            evidence["last_api"] = reader.last
        if observer is not None:
            evidence["resources"] = observer.finish(validate=False)
        public = {
            key: value
            for key, value in evidence.items()
            if key not in ("screenshots", "states", "source_before", "source_after")
        }
        if len(json.dumps(public).encode()) > 192 * 1024:
            public = {key: evidence[key] for key in ("stage", "summary", "error_type")}
        print("B2_STACK_FAILURE " + json.dumps(public, sort_keys=True), flush=True)
        raise
    finally:
        try:
            if driver is not None:
                driver.quit()
        finally:
            if observer is not None:
                observer.finish(validate=False)
            encoded = json.dumps(evidence, indent=2) + "\n"
            require(
                len(encoded.encode()) <= 2 * 1024 * 1024, "Stack result exceeds bound"
            )
            output.write_text(encoded)


if __name__ == "__main__":
    main()
