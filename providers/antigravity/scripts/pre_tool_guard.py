#!/usr/bin/env python3
"""Antigravity PreToolUse hook: enforce mailbox archive & secret-hygiene rules.

Blocks:
  1. Direct writes or edits to `mailbox.phext` (must use `phext_mailbox.py compact`).
  2. Secret/token patterns written into inbox/ or outbox/ mailbox payloads
     (Protocol Rule 3: No secrets in mailboxes).
"""

from __future__ import annotations

import json
import re
import sys

SECRET_PATTERNS = [
    re.compile(r"x-phext-agent-secret\s*[:=]\s*\S+", re.IGNORECASE),
    re.compile(r"-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----"),
    re.compile(r"\bgh[pousr]_[A-Za-z0-9]{20,}\b"),
    re.compile(r"\bsk-[A-Za-z0-9_-]{20,}\b"),
    re.compile(r"\bpmb-v1-[A-Za-z0-9_-]{8,}\b"),
    re.compile(r"\bBearer\s+[A-Za-z0-9._-]{16,}\b"),
]


def has_secret(text: str) -> bool:
    return any(p.search(text) for p in SECRET_PATTERNS)


def is_mailbox_message_path(path: str) -> bool:
    norm = path.replace("\\", "/").lower()
    return "/outbox/" in norm or "/inbox/" in norm


def deny(reason: str) -> int:
    sys.stdout.write(json.dumps({"decision": "deny", "reason": reason}) + "\n")
    return 0


def allow() -> int:
    sys.stdout.write(json.dumps({"decision": "allow"}) + "\n")
    return 0


def main() -> int:
    try:
        payload = json.load(sys.stdin)
    except Exception:
        return allow()

    tool_call = payload.get("toolCall") or {}
    name = str(tool_call.get("name", ""))
    args = tool_call.get("args") or {}

    path = str(args.get("Path") or args.get("path") or "")
    norm_path = path.replace("\\", "/").lower()
    if name in ("write_file", "edit_file") and (
        norm_path.endswith("/mailbox.phext") or norm_path == "mailbox.phext"
    ):
        return deny(
            "Never hand-edit mailbox.phext; archival is performed exclusively by `phext_mailbox.py compact` (echolocation README)."
        )

    if "echolocation_write_outbox" in name:
        body = str(args.get("body", ""))
        topic = str(args.get("topic", ""))
        re_hdr = str(args.get("re", ""))
        if has_secret("\n".join((topic, re_hdr, body))):
            return deny(
                "Blocked by Protocol Rule 3 (No secrets in mailboxes): detected credential or token pattern in outbox payload."
            )
    elif name in ("write_file", "edit_file") and is_mailbox_message_path(path):
        content = str(args.get("Content") or args.get("Replacement") or "")
        if has_secret(content):
            return deny(
                "Blocked by Protocol Rule 3 (No secrets in mailboxes): detected credential or token pattern in mailbox file."
            )

    return allow()


if __name__ == "__main__":
    raise SystemExit(main())
