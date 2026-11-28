#!/usr/bin/env python3
"""Unit tests for the Antigravity echolocation plugin hooks and MCP server."""

from __future__ import annotations

import http.server
import json
import os
import subprocess
import sys
import tempfile
import threading
import unittest
import urllib.parse
from pathlib import Path

SCRIPTS_DIR = Path(__file__).resolve().parent


class TestAntigravityPlugin(unittest.TestCase):
    def _run_hook(self, script: str, payload: dict, env_extra: dict | None = None) -> dict:
        env = os.environ.copy()
        if env_extra:
            env.update(env_extra)
        proc = subprocess.run(
            [sys.executable, str(SCRIPTS_DIR / script)],
            input=json.dumps(payload),
            text=True,
            capture_output=True,
            env=env,
            check=True,
        )
        return json.loads(proc.stdout)

    def test_session_start_drain_filters_expired_and_fences_untrusted(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            inbox = Path(tmp) / "inbox"
            inbox.mkdir()
            (inbox / "20260101T000000Z-old-stale.md").write_text(
                "from: old\nre: stale\nexpires: 2020-01-01T00:00:00Z\n\nstale body\n",
                encoding="utf-8",
            )
            (inbox / "20261127T230000Z-mira-hello.md").write_text(
                "from: mira\nre: hello\nexpires: 2099-01-01T00:00:00Z\nrespond-by: 2099-01-01T00:00:00Z\n\nhello pod\n",
                encoding="utf-8",
            )
            out = self._run_hook(
                "session_start_drain.py",
                {"conversationId": "test", "hookEventName": "SessionStart"},
                {"ECHOLOCATION_MAILBOX_DIR": tmp},
            )
            msg = out["injectSteps"][0]["systemMessage"]["systemMessage"]
            self.assertIn("UNTRUSTED MAILBOX DATA", msg)
            self.assertIn("1 live, 1 expired skipped", msg)
            self.assertIn("20261127T230000Z-mira-hello.md", msg)
            self.assertNotIn("stale body", msg)

    def test_pre_tool_guard_blocks_mailbox_phext_and_secrets(self) -> None:
        # Block direct edit to mailbox.phext
        res = self._run_hook(
            "pre_tool_guard.py",
            {"toolCall": {"name": "write_file", "args": {"Path": "/chat/seat/mailbox.phext", "Content": "x"}}},
        )
        self.assertEqual(res["decision"], "deny")

        # Allow read_file on mailbox.phext
        res = self._run_hook(
            "pre_tool_guard.py",
            {"toolCall": {"name": "read_file", "args": {"Path": "mailbox.phext"}}},
        )
        self.assertEqual(res["decision"], "allow")

        # Block secret in outbox MCP write
        res = self._run_hook(
            "pre_tool_guard.py",
            {
                "toolCall": {
                    "name": "mcp_echolocation_echolocation_write_outbox",
                    "args": {"topic": "t", "re": "r", "body": "key is pmb-v1-secret12345"},
                }
            },
        )
        self.assertEqual(res["decision"], "deny")

        # Allow clean outbox write
        res = self._run_hook(
            "pre_tool_guard.py",
            {
                "toolCall": {
                    "name": "mcp_echolocation_echolocation_write_outbox",
                    "args": {"topic": "t", "re": "r", "body": "clean status reply"},
                }
            },
        )
        self.assertEqual(res["decision"], "allow")

    def test_mcp_server_mailbox_and_sq_tools(self) -> None:
        store: dict[str, str] = {}

        class MockSQ(http.server.BaseHTTPRequestHandler):
            def log_message(self, fmt: str, *args: object) -> None:
                return

            def do_GET(self) -> None:
                parsed = urllib.parse.urlparse(self.path)
                q = urllib.parse.parse_qs(parsed.query)
                c = q.get("c", [""])[0]
                body = store.get(c, "").encode("utf-8")
                self.send_response(200)
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

            def do_POST(self) -> None:
                parsed = urllib.parse.urlparse(self.path)
                q = urllib.parse.parse_qs(parsed.query)
                c = q.get("c", [""])[0]
                length = int(self.headers.get("Content-Length", "0"))
                store[c] = self.rfile.read(length).decode("utf-8")
                body = b"Updated"
                self.send_response(200)
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

        httpd = http.server.ThreadingHTTPServer(("127.0.0.1", 0), MockSQ)
        port = httpd.server_address[1]
        t = threading.Thread(target=httpd.serve_forever, daemon=True)
        t.start()

        try:
            with tempfile.TemporaryDirectory() as tmp:
                env = os.environ.copy()
                env.update(
                    {
                        "ECHOLOCATION_SEAT": "agy-test",
                        "ECHOLOCATION_MAILBOX_DIR": tmp,
                        "ECHOLOCATION_SQ_URL": f"http://127.0.0.1:{port}",
                        "ECHOLOCATION_SQ_PHEXT": "echolocation",
                        "ECHOLOCATION_SQ_WRITABLE": "1",
                    }
                )
                requests = [
                    {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}},
                    {"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}},
                    {
                        "jsonrpc": "2.0",
                        "id": 3,
                        "method": "tools/call",
                        "params": {
                            "name": "echolocation_write_outbox",
                            "arguments": {
                                "topic": "hello",
                                "re": "intro",
                                "body": "ready in pod",
                                "expires_utc": "2099-01-02T00:00:00Z",
                                "respond_by_utc": "2099-01-01T00:00:00Z",
                            },
                        },
                    },
                    {
                        "jsonrpc": "2.0",
                        "id": 4,
                        "method": "tools/call",
                        "params": {
                            "name": "echolocation_write_outbox",
                            "arguments": {
                                "topic": "bad-timing",
                                "re": "intro",
                                "body": "should fail",
                                "expires_utc": "2099-01-01T00:00:00Z",
                                "respond_by_utc": "2099-01-02T00:00:00Z",
                            },
                        },
                    },
                    {
                        "jsonrpc": "2.0",
                        "id": 5,
                        "method": "tools/call",
                        "params": {
                            "name": "echolocation_sq_update",
                            "arguments": {"coordinate": "1.1.1/1.1.1/9.1.1", "text": "echolocation/signature 1"},
                        },
                    },
                    {
                        "jsonrpc": "2.0",
                        "id": 6,
                        "method": "tools/call",
                        "params": {
                            "name": "echolocation_sq_select",
                            "arguments": {"coordinate": "1.1.1/1.1.1/9.1.1"},
                        },
                    },
                ]
                proc = subprocess.run(
                    [sys.executable, str(SCRIPTS_DIR / "echolocation_mcp.py")],
                    input="\n".join(json.dumps(r) for r in requests) + "\n",
                    text=True,
                    capture_output=True,
                    env=env,
                    check=True,
                )
                responses = [json.loads(line) for line in proc.stdout.splitlines() if line.strip()]
                self.assertEqual(len(responses), 6)
                self.assertEqual(responses[0]["result"]["serverInfo"]["name"], "echolocation")
                self.assertEqual(len(responses[1]["result"]["tools"]), 5)

                write_ok = json.loads(responses[2]["result"]["content"][0]["text"])
                self.assertTrue(write_ok["verified"])
                self.assertIn("-agy-test-hello.md", write_ok["filename"])

                # respond-by > expires must return isError=True
                self.assertTrue(responses[3]["result"]["isError"])

                sq_write = json.loads(responses[4]["result"]["content"][0]["text"])
                self.assertTrue(sq_write["verified"])

                sq_read = json.loads(responses[5]["result"]["content"][0]["text"])
                self.assertTrue(sq_read["untrusted_data"])
                self.assertEqual(sq_read["content"], "echolocation/signature 1")
        finally:
            httpd.shutdown()
            httpd.server_close()


if __name__ == "__main__":
    unittest.main()
