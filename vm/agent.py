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


def log(msg: str) -> None:
    """Write a log line to stderr so it appears in server logs without polluting the stdout protocol."""
    sys.stderr.write(f"[agent] {msg}\n")
    sys.stderr.flush()


async def main():
    loop = asyncio.get_event_loop()
    reader = asyncio.StreamReader()
    proto = asyncio.StreamReaderProtocol(reader)
    await loop.connect_read_pipe(lambda: proto, sys.stdin)
    log("ready, waiting for queries")
    while True:
        raw = await reader.readline()
        if not raw:
            log("stdin closed, exiting")
            break
        line = raw.decode().strip()
        if not line:
            continue
        try:
            msg = json.loads(line)
        except json.JSONDecodeError:
            log(f"failed to parse stdin line: {line[:120]}")
            continue
        if msg.get('type') == 'query':
            await run_query(msg.get('content', ''), msg.get('session_id'))


async def run_query(content: str, session_id):
    from claude_agent_sdk import ClaudeAgentOptions, query
    from claude_agent_sdk.types import StreamEvent

    preview = content[:80].replace('\n', ' ')
    log(f"query start  session_id={session_id!r}  content={preview!r}")

    options = ClaudeAgentOptions(
        cwd=os.environ.get('HOME', '/root'),
        permission_mode='bypassPermissions',
        **({"resume": session_id} if session_id else {}),
    )
    captured_session_id = session_id
    try:
        async for event in query(prompt=content, options=options):
            if isinstance(event, StreamEvent):
                ev = event.event
                ev_type = getattr(ev, 'type', None) or (ev.get('type') if isinstance(ev, dict) else None)
                # Only log structural stream events, not text deltas (too noisy)
                if ev_type not in ('content_block_delta', 'message_delta'):
                    log(f"stream_event  {ev_type}")
                emit(
                    {
                        "type": "stream_event",
                        "session_id": event.session_id,
                        "event": event.event,
                    }
                )
            else:
                event_type = getattr(event, 'type', None)
                if event_type == 'assistant':
                    content_blocks = []
                    msg = getattr(event, 'message', None)
                    if msg:
                        raw_content = getattr(msg, 'content', [])
                        content_blocks = [getattr(b, 'type', '?') for b in (raw_content or [])]
                    log(f"assistant  blocks={content_blocks}  session_id={getattr(event, 'session_id', None)!r}")
                elif event_type == 'user':
                    msg = getattr(event, 'message', None)
                    if msg:
                        raw_content = getattr(msg, 'content', []) or []
                        tool_ids = [getattr(b, 'tool_use_id', None) for b in raw_content if getattr(b, 'type', None) == 'tool_result']
                        log(f"user tool_results  tool_use_ids={tool_ids}")
                elif event_type == 'result':
                    log(f"result  subtype={getattr(event, 'subtype', '?')}  session_id={getattr(event, 'session_id', None)!r}")
                elif event_type == 'system':
                    log(f"system  subtype={getattr(event, 'subtype', '?')}  session_id={getattr(event, 'session_id', None)!r}")
                elif event_type:
                    log(f"event  type={event_type!r}")
                emit(event)
            if hasattr(event, 'session_id') and event.session_id:
                captured_session_id = event.session_id
    except Exception as exc:
        log(f"query error: {exc}")
        emit({'type': 'error', 'message': str(exc)})
    log(f"query done  session_id={captured_session_id!r}")
    emit({'type': 'done', 'session_id': captured_session_id})


class _Encoder(json.JSONEncoder):
    def default(self, obj):
        if dataclasses.is_dataclass(obj) and not isinstance(obj, type):
            data = dataclasses.asdict(obj)
        elif hasattr(obj, 'model_dump'):
            data = obj.model_dump()
        else:
            return super().default(obj)
        # Pydantic model_dump() may omit discriminator `type`; re-inject from attribute if missing.
        if isinstance(data, dict) and 'type' not in data:
            type_val = getattr(obj, 'type', None)
            if type_val is not None:
                data['type'] = type_val if isinstance(type_val, str) else str(type_val)
        return data


def emit(obj):
    if dataclasses.is_dataclass(obj) and not isinstance(obj, type):
        data = dataclasses.asdict(obj)
    elif hasattr(obj, 'model_dump'):
        data = obj.model_dump()
    elif isinstance(obj, dict):
        data = obj
    else:
        data = {'raw': str(obj)}
    # Pydantic's model_dump() may omit discriminator `type` fields; re-inject from attribute if missing.
    if isinstance(data, dict) and 'type' not in data:
        type_val = getattr(obj, 'type', None)
        if type_val is not None:
            data['type'] = type_val if isinstance(type_val, str) else str(type_val)
    sys.stdout.write(json.dumps(data, cls=_Encoder) + '\n')
    sys.stdout.flush()


asyncio.run(main())
