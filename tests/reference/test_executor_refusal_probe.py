"""The installed UID-refusal probe must handle a peer closing before send."""

import ast
import importlib.util
import io
import json
import socket
import struct
import sys
import unittest
from contextlib import redirect_stdout
from pathlib import Path
from unittest.mock import patch

SOURCE = Path(__file__).resolve().parents[1] / "privileged_vm/guest.py"
SPEC = importlib.util.spec_from_file_location("foundation_refusal_fixture", SOURCE)
FIXTURE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(FIXTURE)


def probe_code():
    values = [
        node.value.value
        for node in ast.walk(ast.parse(SOURCE.read_text()))
        if isinstance(node, ast.Assign)
        and any(
            isinstance(target, ast.Name) and target.id == "probe"
            for target in node.targets
        )
        and isinstance(node.value, ast.Constant)
        and isinstance(node.value.value, str)
    ]
    assert len(values) == 1
    return values[0]


def frame(value):
    body = json.dumps(value).encode()
    return struct.pack("!I", len(body)) + body


class ExecutorRefusalProbeTests(unittest.TestCase):
    def execute(
        self,
        reply=b"",
        *,
        close_peer=True,
        send_error=None,
        connect_error=None,
        recv_error=None,
    ):
        # A real connected Unix pair makes pre-send EPIPE deterministic. Only
        # connect is substituted: transport refusals are not UID-auth proof.
        client, peer = socket.socketpair(socket.AF_UNIX)
        self.addCleanup(client.close)
        self.addCleanup(peer.close)
        if reply:
            peer.sendall(reply)
        if close_peer:
            peer.close()

        class Connection:
            def settimeout(self, timeout):
                client.settimeout(timeout)

            def connect(self, _):
                if connect_error:
                    raise connect_error

            def sendall(self, data):
                if send_error:
                    raise send_error
                client.sendall(data)

            def recv(self, size):
                if recv_error:
                    raise recv_error
                return client.recv(size)

        output = io.StringIO()
        with (
            patch.object(socket, "socket", return_value=Connection()),
            patch.object(
                sys,
                "argv",
                ["probe", "/private-fixture", '{"operation":"health","version":1}'],
            ),
            redirect_stdout(output),
        ):
            try:
                exec(probe_code(), {})  # noqa: S102 - exercise the repository's fixed IPC probe
            except SystemExit as error:
                self.assertEqual(error.code, 0)
        return output.getvalue()

    def test_real_peer_close_before_send_is_a_refusal(self):
        self.assertEqual(self.execute(), "denied\n")
        self.assertEqual(
            self.execute(send_error=ConnectionResetError("reset write")), "denied\n"
        )
        self.assertEqual(
            self.execute(recv_error=ConnectionResetError("reset header")), "denied\n"
        )

    def test_buffered_explicit_forbidden_is_read_after_pre_send_close(self):
        reply = {"version": 1, "ready": False, "error": "forbidden"}
        result = self.execute(frame(reply))
        self.assertEqual(json.loads(result), reply)
        self.assertTrue(FIXTURE.refused_executor_reply(result, "forbidden"))

    def test_buffered_health_success_never_qualifies_as_denial(self):
        result = self.execute(frame({"version": 1, "ready": True}))
        self.assertFalse(FIXTURE.refused_executor_reply(result, "forbidden"))

    def test_authorized_health_and_exact_invalid_input_remain_explicit(self):
        ready = self.execute(frame({"version": 1, "ready": True}), close_peer=False)
        self.assertTrue(json.loads(ready)["ready"])
        invalid = {"version": 1, "ready": False, "error": "invalid_input"}
        result = self.execute(frame(invalid), close_peer=False)
        self.assertTrue(FIXTURE.refused_executor_reply(result, "invalid_input"))
        self.assertFalse(FIXTURE.refused_executor_reply(result, "forbidden"))

    def test_partial_header_and_truncated_body_still_fail(self):
        for reply, error in [
            (b"bad", struct.error),
            (struct.pack("!I", 10) + b"short", RuntimeError),
        ]:
            with self.subTest(reply=reply), self.assertRaises(error):
                self.execute(reply)

    def test_connect_timeout_and_unexpected_write_error_are_not_denials(self):
        for kwargs, expected in [
            ({"connect_error": PermissionError("denied connect")}, PermissionError),
            ({"send_error": TimeoutError("stalled write")}, TimeoutError),
            ({"send_error": OSError(5, "I/O fault")}, OSError),
            ({"recv_error": TimeoutError("stalled header")}, TimeoutError),
            ({"recv_error": OSError(5, "header I/O fault")}, OSError),
        ]:
            with self.subTest(kwargs=kwargs), self.assertRaises(expected):
                self.execute(close_peer=False, **kwargs)
