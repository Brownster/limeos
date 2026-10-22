"""Reference capture isolation and reviewed frozen notification settings."""

import json
import sys
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

import frozen_notifications as frozen

FIXTURE = (
    Path(__file__).resolve().parents[1]
    / "fixtures/automation/frozen-notification-settings.json"
)


class CaptureIsolationTests(unittest.TestCase):
    def test_only_selected_definitions_execute_and_module_registration_is_restored(
        self,
    ):
        before = set(sys.modules)
        source = 'raise RuntimeError("module setup must not execute")\ndef selected():\n    return 42\n'
        with frozen.definitions(source, ("selected",)) as values:
            self.assertEqual(values["selected"](), 42)
        self.assertEqual(set(sys.modules), before)

    def test_real_imports_are_not_available_to_extracted_definitions(self):
        source = "def selected():\n    import socket\n    return socket.socket()\n"
        with frozen.definitions(source, ("selected",)) as values:
            with self.assertRaisesRegex(ImportError, "undeclared import"):
                values["selected"]()

    def test_duplicate_missing_and_unlisted_source_selections_are_refused(self):
        for source in ("def duplicate(): pass\ndef duplicate(): pass", "other = 1"):
            with self.assertRaises(ValueError):
                frozen.select(source, "duplicate")
        with self.assertRaises(ValueError):
            frozen.git_blob(Path("unused"), "config/instance-secrets.env")

    def test_git_read_pins_the_blob_and_refuses_oversize_source(self):
        with patch.object(
            frozen.subprocess, "run", return_value=SimpleNamespace(stdout=b"source")
        ) as command:
            self.assertEqual(
                frozen.git_blob(Path("fixture"), "alert_policy.py"), b"source"
            )
        self.assertIn(
            f"{frozen.REFERENCE_COMMIT}:alert_policy.py", command.call_args.args[0]
        )
        self.assertIn("--no-optional-locks", command.call_args.args[0])
        with patch.object(
            frozen.subprocess,
            "run",
            return_value=SimpleNamespace(stdout=b"x" * (frozen.SOURCE_LIMIT + 1)),
        ):
            with self.assertRaises(ValueError):
                frozen.git_blob(Path("fixture"), "alert_policy.py")


class ReviewedNotificationFixtures(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.fixture = json.loads(FIXTURE.read_text())
        cls.cases = {case["id"]: case["output"] for case in cls.fixture["cases"]}

    def test_reference_identity_unique_cases_and_no_fixture_credentials(self):
        self.assertEqual(self.fixture["reference_commit"], frozen.REFERENCE_COMMIT)
        self.assertEqual(len(self.cases), len(self.fixture["cases"]))
        self.assertEqual(len(self.cases), 32)
        self.assertEqual(
            {row["path"] for row in self.fixture["source_inputs"]}, set(frozen.SOURCES)
        )
        for secret in (
            "fixture-webhook",
            "fixture-alerts",
            "fixture-updates",
            "fixture-existing-token",
            "fixture-generated-token",
            "fixture-destination",
        ):
            self.assertNotIn(secret, json.dumps(self.fixture))

    def test_health_defaults_and_explicitly_unsupported_legacy_bounds_are_recorded(
        self,
    ):
        self.assertEqual(self.cases["daemon-defaults"]["poll_seconds"], 60)
        self.assertEqual(self.cases["daemon-defaults"]["fail_threshold"], 2)
        self.assertEqual(self.cases["daemon-clamps-zero"]["poll_seconds"], 1)
        self.assertEqual(self.cases["daemon-large-threshold"]["fail_threshold"], 100)
        self.assertEqual(self.cases["daemon-invalid-integers"]["fail_threshold"], 2)

    def test_policy_precedence_preserves_explicit_empty_and_literal_case(self):
        self.assertEqual(
            self.cases["mount-env-with-missing-policy-file"],
            {"value": ["/mnt/Required"]},
        )
        self.assertEqual(self.cases["empty-policy-overrides-env-mounts"], {"value": []})
        self.assertEqual(
            self.cases["wrapped-policy-overrides-env-mounts"], {"value": ["/srv/Array"]}
        )
        self.assertEqual(
            self.cases["policy-literal-mounts"]["value"]["required_mounts"],
            ["/mnt/Media", "/mnt/media"],
        )

    def test_expiry_is_half_open_and_operator_silence_metadata_is_retained(self):
        self.assertEqual(self.cases["silence-before-expiry"], {"allows": False})
        self.assertEqual(self.cases["silence-at-expiry"], {"allows": True})
        silence = self.cases["policy-silence-metadata"]["value"]["silences"][0]
        self.assertEqual(silence["reason"], "disk maintenance")
        self.assertEqual(silence["created_at"], "2026-10-20T12:00:00+00:00")

    def test_disabled_dedicated_channel_is_not_a_global_disable(self):
        self.assertEqual(
            self.cases["updates-dedicated-first"], {"value": {"destination": "updates"}}
        )
        self.assertEqual(
            self.cases["updates-disabled-dedicated-falls-back"],
            {"value": {"destination": "alerts"}},
        )
        self.assertEqual(
            self.cases["updates-malformed-config-falls-back"],
            {"value": {"destination": "alerts"}},
        )
        self.assertEqual(
            self.cases["updates-no-destination"], {"value": {"destination": None}}
        )

    def test_provisioning_reenables_but_preserves_existing_ingest_identity(self):
        self.assertEqual(self.cases["stack-default-provision"]["mode"], "quiet")
        reprovision = self.cases["stack-reprovision-preserves-settings"]
        self.assertTrue(reprovision["enabled"])
        self.assertEqual(reprovision["mode"], "verbose")
        self.assertEqual(reprovision["ingest_token"], "preserved")
        self.assertEqual(reprovision["file_mode"], "0600")

    def test_saved_notify_flag_does_not_prove_an_implemented_sender(self):
        self.assertEqual(
            self.cases["container-notify-setting-has-no-run-consumer"],
            {"default": True, "referenced_by_run": False},
        )
        self.assertEqual(
            self.cases["updates-scalar-config-errors"], {"error": "AttributeError"}
        )


if __name__ == "__main__":
    unittest.main()
