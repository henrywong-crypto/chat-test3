#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///

import json
import sys
from pathlib import Path

SETTINGS_PATH = Path.home() / ".claude" / "settings.json"


def get_settings() -> dict:
    if not SETTINGS_PATH.exists():
        return {"has_api_key": False}
    try:
        data = json.loads(SETTINGS_PATH.read_text())
    except (json.JSONDecodeError, OSError):
        return {"has_api_key": False}
    env = data.get("env", {})
    has_api_key = bool(env.get("ANTHROPIC_AUTH_TOKEN"))
    if has_api_key:
        env = {k: ("***" if k == "ANTHROPIC_AUTH_TOKEN" else v) for k, v in env.items()}
        data = {**data, "env": env}
    return {**data, "has_api_key": has_api_key}


def set_settings(content: str) -> dict:
    SETTINGS_PATH.parent.mkdir(parents=True, exist_ok=True)
    SETTINGS_PATH.write_text(content)
    return {"ok": True}


def main() -> None:
    raw = sys.stdin.readline()
    if not raw:
        return
    try:
        msg = json.loads(raw.strip())
    except json.JSONDecodeError:
        print(json.dumps({"error": "invalid json"}))
        return
    cmd_type = msg.get("type")
    if cmd_type == "get":
        print(json.dumps(get_settings()))
    elif cmd_type == "set":
        content = msg.get("content", "")
        print(json.dumps(set_settings(content)))
    else:
        print(json.dumps({"error": f"unknown type: {cmd_type}"}))


main()
