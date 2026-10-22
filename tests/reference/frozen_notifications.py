# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Capture selected pinned notification rules with in-memory dependencies only.

Never imports pihealth_helper or starts the frozen app. Git blob reads are pinned
to the frozen revision; fixture credentials are represented by named slots.
This is a reference harness for trusted source, not a general Python sandbox.
"""

import __future__

import ast
import builtins
import hashlib
import io
import json
import subprocess
import sys
from collections.abc import Mapping
from contextlib import contextmanager
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path, PurePosixPath
from types import ModuleType, SimpleNamespace

REFERENCE_COMMIT = "80593b29443ea9b81eb60ed4e5c3c3ac7a236204"
SOURCE_LIMIT = 1024 * 1024
SOURCES = (
    "alert_daemon.py",
    "alert_policy.py",
    "alert_evaluator.py",
    "alert_notifier.py",
    "alert_history.py",
    "stack_notifications_service.py",
    "mattermost_integration_service.py",
    "pihealth_helper.py",
    "limeos_packages.py",
    "update_service.py",
    "update_scheduler.py",
    "backup_scheduler.py",
    "integrations_manager.py",
    "runtime_paths.py",
    "frontend/src/lib/settings.ts",
    "frontend/src/components/integrations/stack-notifications-card.tsx",
    "app.py",
    "setup_manager.py",
    "integration_capability_adapters.py",
    "capability_registry_service.py",
    "frontend/src/lib/integrations.ts",
    "frontend/src/pages/integrations-page.tsx",
    "deploy/alertd/limeos-alertd.service",
    "deploy/alertd/compose.sidecar.yaml",
    "agent_automation/reporting.py",
)


def git_blob(repository: Path, path: str) -> bytes:
    """Read only a named source blob, independent of working-tree configuration."""
    if path not in SOURCES:
        raise ValueError("Reference source is outside the declared list")
    result = subprocess.run(
        [
            "git",
            "--no-optional-locks",
            "-C",
            str(repository),
            "show",
            f"{REFERENCE_COMMIT}:{path}",
        ],
        check=True,
        capture_output=True,
        timeout=10,
    )
    if len(result.stdout) > SOURCE_LIMIT:
        raise ValueError("Reference source exceeds the extraction bound")
    return result.stdout


def select(source: str, qualified: str):
    """Select a definition or literal assignment without executing module setup."""
    nodes = ast.parse(source).body
    for part in qualified.split("."):
        found = [
            node
            for node in nodes
            if isinstance(node, (ast.ClassDef, ast.FunctionDef)) and node.name == part
        ]
        if not found and "." not in qualified:
            found = [
                node
                for node in nodes
                if isinstance(node, ast.Assign)
                and any(isinstance(t, ast.Name) and t.id == part for t in node.targets)
            ]
        if len(found) != 1:
            raise ValueError(f"Expected one selected definition: {qualified}")
        chosen = found[0]
        nodes = chosen.body if isinstance(chosen, ast.ClassDef) else []
    return chosen


@contextmanager
def definitions(source: str, names: tuple[str, ...], supplied=None):
    """Execute only the specified frozen definitions against fixture bindings."""
    selected = [select(source, name) for name in names]
    module_name = (
        "_limeos_frozen_notifications_"
        + hashlib.sha256((source + repr(names)).encode()).hexdigest()[:16]
    )
    module = ModuleType(module_name)
    previous = sys.modules.get(module_name)
    allowed = {
        name: getattr(builtins, name)
        for name in (
            "__build_class__",
            "object",
            "str",
            "int",
            "bool",
            "dict",
            "list",
            "tuple",
            "set",
            "isinstance",
            "len",
            "max",
            "any",
            "classmethod",
            "staticmethod",
            "property",
            "Exception",
            "ValueError",
            "TypeError",
            "OSError",
            "FileNotFoundError",
        )
    }

    def fixture_import(name, _globals=None, _locals=None, _fromlist=(), level=0):
        if name == "runtime_paths" and level == 0:
            return SimpleNamespace(STATE_DIR="/fixture/limeos/state")
        raise ImportError("Frozen extraction attempted an undeclared import")

    allowed["__import__"] = fixture_import
    module.__dict__.update(
        {
            "__builtins__": allowed,
            "dataclass": dataclass,
            "Mapping": Mapping,
            "datetime": datetime,
            "timezone": timezone,
            "Path": PurePosixPath,
            "json": json,
            "os": SimpleNamespace(environ={}),
        }
    )
    module.__dict__.update(
        {
            key: value
            for key, value in (supplied or {}).items()
            if not key.startswith("__")
        }
    )
    sys.modules[module_name] = module
    try:
        code = compile(
            ast.Module(body=selected, type_ignores=[]),
            "frozen-notification-definitions",
            "exec",
            flags=__future__.annotations.compiler_flag,
            dont_inherit=True,
        )
        exec(code, module.__dict__)
        yield module.__dict__
    finally:
        if previous is None:
            del sys.modules[module_name]
        else:
            sys.modules[module_name] = previous


def outcome(call):
    """Return data or a bounded exception type, never exception text or secrets."""
    try:
        return {"value": call()}
    except (ValueError, TypeError, AttributeError, OSError) as error:
        return {"error": type(error).__name__}


class MemoryPath:
    def __init__(self, contents=None, error=None):
        self.contents = contents
        self.error = error

    def read_text(self):
        if self.error:
            raise self.error
        return self.contents


class RecordingRepository:
    def __init__(self, existing):
        self.existing = existing
        self.written = None

    def read_json(self, _path, *, default):
        return self.existing if self.existing is not None else default

    def write_json(self, _path, value, *, mode):
        self.written = (value, mode)


def capture(repository: Path) -> dict:
    blobs = {name: git_blob(repository, name) for name in SOURCES}
    texts = {name: data.decode() for name, data in blobs.items()}
    cases = []
    with definitions(
        texts["alert_daemon.py"], ("DaemonConfig", "config_from_env")
    ) as env:
        for name, values in (
            ("daemon-defaults", {}),
            (
                "daemon-custom",
                {
                    "LIMEOS_ALERT_MATTERMOST_WEBHOOK": "fixture-webhook",
                    "LIMEOS_ALERT_POLL_SECONDS": "30",
                    "LIMEOS_ALERT_FAIL_THRESHOLD": "3",
                    "LIMEOS_ALERT_REQUIRED_MOUNTS": "/mnt/Media, /mnt/media, /mnt/Media",
                },
            ),
            (
                "daemon-invalid-integers",
                {
                    "LIMEOS_ALERT_POLL_SECONDS": "bad",
                    "LIMEOS_ALERT_FAIL_THRESHOLD": "bad",
                },
            ),
            (
                "daemon-clamps-zero",
                {"LIMEOS_ALERT_POLL_SECONDS": "0", "LIMEOS_ALERT_FAIL_THRESHOLD": "-1"},
            ),
            ("daemon-large-threshold", {"LIMEOS_ALERT_FAIL_THRESHOLD": "100"}),
        ):
            value = env["config_from_env"](values)
            projected = {
                key: "<present-secret>"
                if key == "LIMEOS_ALERT_MATTERMOST_WEBHOOK"
                else item
                for key, item in values.items()
            }
            cases.append(
                {
                    "id": name,
                    "input": {"environment": projected},
                    "output": {
                        "configured": value.webhook_url is not None,
                        "poll_seconds": value.poll_seconds,
                        "fail_threshold": value.fail_threshold,
                        "required_mounts": list(value.required_mounts),
                        "state_path": str(value.state_path),
                        "history_path": str(value.history_path),
                    },
                }
            )

    policy_names = (
        "ALERT_KINDS",
        "default_alert_policy",
        "AlertPolicyError",
        "normalize_alert_policy",
        "_normalize_silence",
        "_parse_time",
        "AlertPolicy",
    )
    with definitions(texts["alert_policy.py"], policy_names) as policy:
        silence = {
            "kind": "mount",
            "key": "mount:/mnt/Media",
            "created_at": "2026-10-20T12:00:00+00:00",
            "expires_at": "2026-10-20T13:00:00+00:00",
            "reason": "  disk maintenance  ",
        }
        for name, raw in (
            ("policy-defaults", {}),
            (
                "policy-disabled-category",
                {"categories": {"container": {"enabled": False}}},
            ),
            (
                "policy-literal-mounts",
                {"required_mounts": ["/mnt/Media", "/mnt/media", "/mnt/Media"]},
            ),
            ("policy-silence-metadata", {"silences": [silence]}),
            (
                "policy-string-enabled-refused",
                {"categories": {"container": {"enabled": "false"}}},
            ),
            ("policy-relative-mount-refused", {"required_mounts": ["relative/path"]}),
            ("policy-threshold-is-not-a-policy-field", {"fail_threshold": 9}),
        ):
            cases.append(
                {
                    "id": name,
                    "input": raw,
                    "output": outcome(
                        lambda raw=raw: policy["normalize_alert_policy"](raw)
                    ),
                }
            )
        value = policy["AlertPolicy"].from_mapping({"silences": [silence]})
        for name, at in (
            ("silence-before-expiry", "2026-10-20T12:59:59+00:00"),
            ("silence-at-expiry", "2026-10-20T13:00:00+00:00"),
        ):
            cases.append(
                {
                    "id": name,
                    "output": {
                        "allows": value.allows(
                            "mount", "mount:/mnt/Media", now=datetime.fromisoformat(at)
                        )
                    },
                }
            )
        with definitions(texts["alert_daemon.py"], ("_load_policy",), policy) as daemon:
            for name, body, error in (
                ("mount-env-without-policy-file", None, None),
                ("mount-env-with-missing-policy-file", None, FileNotFoundError()),
                ("mount-env-with-malformed-policy-file", "{", None),
                ("empty-policy-overrides-env-mounts", "{}", None),
                (
                    "wrapped-policy-overrides-env-mounts",
                    '{"policy":{"required_mounts":["/srv/Array"]}}',
                    None,
                ),
                ("null-policy-falls-back-to-env", "null", None),
                ("scalar-policy-errors", "true", None),
            ):
                path = (
                    None if body is None and error is None else MemoryPath(body, error)
                )
                config = SimpleNamespace(
                    policy_path=path, required_mounts=("/mnt/Required",)
                )
                cases.append(
                    {
                        "id": name,
                        "input": {
                            "policy_json": body,
                            "read_error": type(error).__name__ if error else None,
                            "environment_required_mounts": ["/mnt/Required"],
                        },
                        "output": outcome(
                            lambda config=config: list(
                                daemon["_load_policy"](config).required_mounts
                            )
                        ),
                    }
                )

    for name, body, alerts in (
        (
            "updates-dedicated-first",
            '{"enabled":true,"webhook_url":"fixture-updates"}',
            True,
        ),
        (
            "updates-disabled-dedicated-falls-back",
            '{"enabled":false,"webhook_url":"fixture-updates"}',
            True,
        ),
        (
            "updates-empty-destination-falls-back",
            '{"enabled":true,"webhook_url":""}',
            True,
        ),
        ("updates-missing-config-falls-back", None, True),
        ("updates-malformed-config-falls-back", "{", True),
        ("updates-no-destination", None, False),
        ("updates-scalar-config-errors", "[]", True),
    ):

        def fixture_open(path, body=body):
            if path != "/fixture/package-updates.json":
                raise ValueError("Unexpected fixture read")
            if body is None:
                raise FileNotFoundError
            return io.StringIO(body)

        supplied = {
            "open": fixture_open,
            "PACKAGE_UPDATES_CONFIG": "/fixture/package-updates.json",
            "MATTERMOST_SECRETS": "/fixture/secrets",
            "_read_env_value": lambda _path, _key, alerts=alerts: (
                "fixture-alerts" if alerts else None
            ),
        }
        with definitions(
            texts["pihealth_helper.py"], ("_updates_webhook",), supplied
        ) as package:

            def selected():
                url, target = package["_updates_webhook"]()
                if url not in (None, "fixture-alerts", "fixture-updates"):
                    raise ValueError("Destination escaped the fixture slot set")
                return {"destination": target}

            projected = (
                None if body is None else outcome(lambda body=body: json.loads(body))
            )
            if (
                projected
                and isinstance(projected.get("value"), dict)
                and "webhook_url" in projected["value"]
            ):
                projected["value"]["destination_present"] = bool(
                    projected["value"].pop("webhook_url")
                )
            cases.append(
                {
                    "id": name,
                    "input": {
                        "package_config": projected,
                        "alerts_destination_present": alerts,
                    },
                    "output": outcome(selected),
                }
            )

    integration = texts["mattermost_integration_service.py"]
    constants = {
        name: ast.literal_eval(select(integration, name).value)
        for name in (
            "INTEGRATION_VERSION",
            "STACK_NOTIFICATIONS_CHANNEL",
            "PACKAGE_UPDATES_CHANNEL",
        )
    }
    supplied = {
        **constants,
        "secrets": SimpleNamespace(token_urlsafe=lambda _n: "fixture-generated-token"),
    }
    for name, existing in (
        ("stack-default-provision", {}),
        (
            "stack-reprovision-preserves-settings",
            {
                "enabled": False,
                "token": "fixture-existing-token",
                "mode": "verbose",
                "source_default": "library",
            },
        ),
    ):
        repo = RecordingRepository(existing)
        owner = SimpleNamespace(
            _repository=repo,
            _stack_notifications_config_path="/fixture/stack-notifications.json",
        )
        with definitions(
            integration,
            ("MattermostIntegrationService._write_stack_notifications_config",),
            supplied,
        ) as stack:
            stack["_write_stack_notifications_config"](
                owner, webhook_url="fixture-destination"
            )
        value, mode = repo.written
        projected = {
            key: bool(item) if key == "token" else item
            for key, item in existing.items()
        }
        cases.append(
            {
                "id": name,
                "input": {"existing": projected},
                "output": {
                    "version": value["version"],
                    "enabled": value["enabled"],
                    "mode": value["mode"],
                    "source_default": value["source_default"],
                    "channel_name": value["channel_name"],
                    "ingest_token": "preserved"
                    if value["token"] == "fixture-existing-token"
                    else "generated",
                    "file_mode": f"{mode:04o}",
                },
            }
        )
    repo = RecordingRepository({})
    owner = SimpleNamespace(
        _repository=repo, _package_updates_config_path="/fixture/package-updates.json"
    )
    with definitions(
        integration,
        ("MattermostIntegrationService._write_package_updates_config",),
        supplied,
    ) as package:
        package["_write_package_updates_config"](
            owner, webhook_url="fixture-destination"
        )
    value, mode = repo.written
    cases.append(
        {
            "id": "package-default-provision",
            "output": {
                "version": value["version"],
                "enabled": value["enabled"],
                "channel_name": value["channel_name"],
                "file_mode": f"{mode:04o}",
            },
        }
    )
    defaults = ast.literal_eval(
        select(texts["update_service.py"], "DEFAULT_CONFIG").value
    )
    runner = select(texts["update_service.py"], "AutoUpdateService._run_locked")
    cases.append(
        {
            "id": "container-notify-setting-has-no-run-consumer",
            "output": {
                "default": defaults["notify_on_update"],
                "referenced_by_run": any(
                    isinstance(node, ast.Constant) and node.value == "notify_on_update"
                    for node in ast.walk(runner)
                ),
            },
        }
    )
    return {
        "version": 1,
        "reference_commit": REFERENCE_COMMIT,
        "source_inputs": [
            {"path": path, "sha256": hashlib.sha256(data).hexdigest()}
            for path, data in blobs.items()
        ],
        "cases": cases,
        "scope": "Pinned source rules with in-memory dependencies; no frozen app imports, real config reads, host probes or network calls",
    }
