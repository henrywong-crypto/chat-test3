#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = ["claude-code-sdk"]
# ///

import asyncio
import dataclasses
import json
import os
import sys


async def main():
    loop = asyncio.get_event_loop()
    reader = asyncio.StreamReader()
    proto = asyncio.StreamReaderProtocol(reader)
    await loop.connect_read_pipe(lambda: proto, sys.stdin)
    while True:
        raw = await reader.readline()
        if not raw:
            break
        line = raw.decode().strip()
        if not line:
            continue
        try:
            msg = json.loads(line)
        except json.JSONDecodeError:
            continue
        if msg.get('type') == 'query':
            await run_query(msg.get('content', ''), msg.get('session_id'))


async def run_query(content: str, session_id):
    from claude_code_sdk import query, ClaudeCodeOptions

    options = ClaudeCodeOptions(
        cwd=os.environ.get('HOME', '/root'),
        permission_mode='bypassPermissions',
        **({"resume": session_id} if session_id else {}),
    )
    try:
        async for event in query(prompt=content, options=options):
            emit(event)
    except Exception as exc:
        emit({'type': 'error', 'message': str(exc)})


def emit(obj):
    if dataclasses.is_dataclass(obj) and not isinstance(obj, type):
        data = dataclasses.asdict(obj)
    elif hasattr(obj, 'model_dump'):
        data = obj.model_dump()
    elif isinstance(obj, dict):
        data = obj
    else:
        data = {'raw': str(obj)}
    sys.stdout.write(json.dumps(data) + '\n')
    sys.stdout.flush()


asyncio.run(main())
