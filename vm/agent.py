#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = ["claude-agent-sdk"]
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
    from claude_agent_sdk import ClaudeAgentOptions, query
    from claude_agent_sdk.types import StreamEvent

    options = ClaudeAgentOptions(
        cwd=os.environ.get('HOME', '/root'),
        permission_mode='bypassPermissions',
        **({"resume": session_id} if session_id else {}),
    )
    captured_session_id = session_id
    try:
        async for event in query(prompt=content, options=options):
            # StreamEvent asdict has no top-level type; frontend expects type stream_event
            if isinstance(event, StreamEvent):
                emit(
                    {
                        "type": "stream_event",
                        "session_id": event.session_id,
                        "event": event.event,
                    }
                )
            else:
                emit(event)
            if hasattr(event, 'session_id') and event.session_id:
                captured_session_id = event.session_id
    except Exception as exc:
        emit({'type': 'error', 'message': str(exc)})
    emit({'type': 'done', 'session_id': captured_session_id})


class _Encoder(json.JSONEncoder):
    def default(self, obj):
        if dataclasses.is_dataclass(obj) and not isinstance(obj, type):
            return dataclasses.asdict(obj)
        if hasattr(obj, 'model_dump'):
            return obj.model_dump()
        return super().default(obj)


def emit(obj):
    if dataclasses.is_dataclass(obj) and not isinstance(obj, type):
        data = dataclasses.asdict(obj)
    elif hasattr(obj, 'model_dump'):
        data = obj.model_dump()
    elif isinstance(obj, dict):
        data = obj
    else:
        data = {'raw': str(obj)}
    sys.stdout.write(json.dumps(data, cls=_Encoder) + '\n')
    sys.stdout.flush()


asyncio.run(main())
