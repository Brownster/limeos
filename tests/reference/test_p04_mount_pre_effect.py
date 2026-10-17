"""Pure refusal-orchestration checks; no API, block device or guest is opened."""

import copy
import importlib.util
import unittest
from pathlib import Path
from unittest import mock

SOURCE = Path(__file__).resolve().parents[1] / "privileged_vm/p04_mounts_guest.py"
SPEC = importlib.util.spec_from_file_location("a1_pre_effect_fixture", SOURCE)
guest = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(guest)


def snapshot():
    return {
        "authority": {
            "jobs": [("prior", "running", "digest", "resource", 1, 123)],
            "claims": [("prior-resource", "prior")],
            "target_receipts": [],
            "target_locks": [],
        },
        "root_receipts_and_claims": [0, 0],
        "source": {"sha256": "original-data", "bytes": 1024},
        "mount_table_sha256": "original-mounts",
        "fstab_sha256": "original-fstab",
        "target": {"inode": 123},
        "configuration_sha256": {},
        "other_data": {},
    }


class PreEffectChecks(unittest.TestCase):
    def test_refusal_requires_exact_status_cause_and_false_retry(self):
        self.assertEqual(
            guest.refusal(
                409, "conflict", (409, {}, {"code": "conflict", "retry": False})
            ),
            {"status": 409, "code": "conflict", "retry": False},
        )
        for status, body in (
            (500, {"code": "conflict", "retry": False}),
            (503, {"code": "unavailable", "retry": True}),
            (409, {"code": "not_found", "retry": False}),
            (409, {"code": "conflict", "retry": "false"}),
        ):
            with self.subTest(status=status, body=body), self.assertRaises(ValueError):
                guest.refusal(409, "conflict", (status, {}, body))

    def test_cancellation_uses_real_returned_approval_before_queue_refusal(self):
        device = {"id": "managed-data"}
        plan = {"id": "plan-id", "effect": {"device": device}}
        proposal = {"plan": plan, "digest": guest.text_digest(guest.canonical(plan))}
        token = "a" * 64
        path = guest.BASE + "/plans/plan-id"
        cancelled = False
        calls = []

        def api(route, method="GET", body=None, cookie=None, csrf=None):
            nonlocal cancelled
            self.assertEqual((method, cookie, csrf), ("POST", "cookie", "csrf"))
            calls.append((route, copy.deepcopy(body)))
            if route == guest.BASE + "/plans":
                if "mountpoint" in body:
                    return 400, {}, {"code": "invalid_input", "retry": False}
                return 201, {}, proposal
            if route == path + "/cancel":
                cancelled = True
                return 204, {}, None
            if cancelled:
                if route == guest.BASE + "/jobs":
                    self.assertEqual(body["proposal"], proposal)
                    self.assertEqual(body["approval"], token)
                else:
                    self.assertEqual(route, path + "/approval")
                return 404, {}, {"code": "not_found", "retry": False}
            self.assertEqual(route, path + "/approval")
            if body["digest"] != proposal["digest"]:
                self.assertEqual(len(body["digest"]), 64)
                return 409, {}, {"code": "conflict", "retry": False}
            return 200, {}, {"token": token}

        before = snapshot()
        with (
            mock.patch.object(guest, "api", side_effect=api),
            mock.patch.object(
                guest, "pre_effect_snapshot", side_effect=lambda _: snapshot()
            ) as observations,
            mock.patch.object(
                guest, "query", return_value=[(1, None, None, proposal["digest"])]
            ) as durable,
        ):
            result = guest.pre_effect_refusals(
                {}, device, "cookie", "csrf", before["authority"]
            )
        self.assertEqual(observations.call_count, 4)
        durable.assert_called_once_with(
            guest.CORE_DB,
            "SELECT canceled,job,approval_digest,digest FROM storage_mount_plans WHERE id=?",
            ("plan-id",),
        )
        self.assertEqual(len(result["cases"]), 3)
        self.assertTrue(all(case["no_effect"] for case in result["cases"]))
        self.assertEqual(
            [route for route, _ in calls[-3:]],
            [path + "/cancel", path + "/approval", guest.BASE + "/jobs"],
        )
        self.assertNotIn(token, guest.canonical(result))

    def test_refusal_cannot_hide_changed_data_or_authority(self):
        before = snapshot()
        for key, change in (
            ("source", {"sha256": "changed-data", "bytes": 1024}),
            ("root_receipts_and_claims", [1, 0]),
            ("fstab_sha256", "changed-fstab"),
            ("mount_table_sha256", "changed-mounts"),
        ):
            after = copy.deepcopy(before)
            after[key] = change
            with self.subTest(changed=key), self.assertRaises(ValueError):
                guest.pre_effect_unchanged(before, after)
        after = copy.deepcopy(before)
        after["authority"]["jobs"].append(("unexpected-job", "queued"))
        with self.assertRaisesRegex(ValueError, "job membership"):
            guest.pre_effect_unchanged(before, after)
        after = copy.deepcopy(before)
        after["authority"]["claims"].append(("new-claim", "prior"))
        with self.assertRaisesRegex(ValueError, "claims changed"):
            guest.pre_effect_unchanged(before, after)


if __name__ == "__main__":
    unittest.main()
