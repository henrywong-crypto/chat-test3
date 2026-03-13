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
import uuid


def log(msg: str) -> None:
    """Write a log line to stderr so it appears in server logs without polluting the stdout protocol."""
    sys.stderr.write(f"[agent] {msg}\n")
    sys.stderr.flush()


def emit_sse(event_name: str, data: dict) -> None:
    """Write a properly formatted SSE event to stdout."""
    sys.stdout.write(f"event: {event_name}\ndata: {json.dumps(data, cls=_Encoder)}\n\n")
    sys.stdout.flush()


def get_field(obj, field, default=None):
    """Get a field from either a dict or an object attribute."""
    if isinstance(obj, dict):
        return obj.get(field, default)
    return getattr(obj, field, default)


# Pending AskUserQuestion futures keyed by request_id.
_pending_questions: dict[str, asyncio.Future] = {}
# Queue for incoming stdin messages (queries, etc.) from the Rust relay.
_stdin_queue: asyncio.Queue = asyncio.Queue()


async def route_stdin(reader: asyncio.StreamReader) -> None:
    """Read stdin lines and route them: answer_question resolves pending futures; everything else goes to _stdin_queue."""
    while True:
        raw = await reader.readline()
        if not raw:
            await _stdin_queue.put(None)
            return
        line = raw.decode().strip()
        if not line:
            continue
        try:
            msg = json.loads(line)
        except json.JSONDecodeError:
            log(f"failed to parse stdin line: {line[:120]}")
            continue
        if msg.get('type') == 'answer_question':
            request_id = msg.get('request_id')
            if request_id and request_id in _pending_questions:
                fut = _pending_questions.pop(request_id)
                if not fut.done():
                    fut.set_result(msg.get('answers', {}))
        else:
            await _stdin_queue.put(msg)


async def main():
    loop = asyncio.get_event_loop()
    reader = asyncio.StreamReader()
    proto = asyncio.StreamReaderProtocol(reader)
    await loop.connect_read_pipe(lambda: proto, sys.stdin)
    log("ready, waiting for queries")
    asyncio.create_task(route_stdin(reader))
    while True:
        msg = await _stdin_queue.get()
        if msg is None:
            log("stdin closed, exiting")
            break
        if msg.get('type') == 'query':
            await run_query(msg.get('content', ''), msg.get('session_id'))


async def run_query(content: str, session_id):
    from claude_agent_sdk import ClaudeAgentOptions, PermissionResultAllow, query
    from claude_agent_sdk.types import StreamEvent

    log(f"query start  session_id={session_id!r}  content_len={len(content)}")

    async def handle_tool_permission(tool_name, input_, context):
        if tool_name != 'AskUserQuestion':
            return PermissionResultAllow()
        request_id = str(uuid.uuid4())
        emit_sse('ask_user_question', {
            'request_id': request_id,
            'questions': input_.get('questions', []),
        })
        fut = asyncio.get_event_loop().create_future()
        _pending_questions[request_id] = fut
        answers = await fut
        log(f"AskUserQuestion answered  request_id={request_id!r}")
        return PermissionResultAllow(updated_input={**input_, 'answers': answers})

    options = ClaudeAgentOptions(
        cwd=os.environ.get('HOME', '/root'),
        can_use_tool=handle_tool_permission,
        **({"resume": session_id} if session_id else {}),
    )

    # can_use_tool requires AsyncIterable prompt (not a plain string).
    async def prompt_stream():
        yield {
            "type": "user",
            "session_id": "",
            "message": {"role": "user", "content": content},
            "parent_tool_use_id": None,
        }

    captured_session_id = session_id
    # Per-block tracking for streaming deltas: index -> type / tool-info / accumulated-input
    block_types: dict[int, str] = {}
    tool_info: dict[int, dict] = {}
    tool_input: dict[int, str] = {}
    # Whether text was already emitted via streaming deltas; if so, skip re-emitting
    # from the full AssistantEvent to avoid duplicates.
    emitted_streaming_text = False
    try:
        async for event in query(prompt=prompt_stream(), options=options):
            if hasattr(event, 'session_id') and event.session_id:
                captured_session_id = event.session_id
            if isinstance(event, StreamEvent):
                had_text = process_stream_event(event, block_types, tool_info, tool_input)
                if had_text:
                    emitted_streaming_text = True
            else:
                process_agent_event(event, emitted_streaming_text)
    except Exception as exc:
        log(f"query error: {exc}")
        emit_sse('error_event', {'message': str(exc)})
    log(f"query done  session_id={captured_session_id!r}")
    emit_sse('done', {'session_id': captured_session_id})


# ── StreamEvent (raw API streaming) ───────────────────────────────────────────

def process_stream_event(
    event,
    block_types: dict,
    tool_info: dict,
    tool_input: dict,
) -> bool:
    """Process a raw API streaming event. Returns True if any text was emitted."""
    ev = event.event
    ev_type = get_field(ev, 'type')
    if ev_type == 'content_block_start':
        return process_block_start(ev, block_types, tool_info, tool_input)
    elif ev_type == 'content_block_delta':
        return process_block_delta(ev, block_types, tool_info, tool_input)
    elif ev_type == 'content_block_stop':
        process_block_stop(ev, block_types, tool_info, tool_input)
    elif ev_type not in ('message_start', 'message_delta', 'message_stop', 'ping', None):
        log(f"stream_event  {ev_type}")
    return False


def process_block_start(ev, block_types: dict, tool_info: dict, tool_input: dict) -> bool:
    idx = get_field(ev, 'index', 0)
    block = get_field(ev, 'content_block')
    block_type = get_field(block, 'type')
    block_types[idx] = block_type
    if block_type == 'text':
        emit_sse('init', {})
        return True
    elif block_type == 'tool_use':
        tool_info[idx] = {'id': get_field(block, 'id'), 'name': get_field(block, 'name')}
        tool_input[idx] = ''
    return False


def process_block_delta(ev, block_types: dict, tool_info: dict, tool_input: dict) -> bool:
    idx = get_field(ev, 'index', 0)
    delta = get_field(ev, 'delta')
    delta_type = get_field(delta, 'type')
    if delta_type == 'text_delta':
        text = get_field(delta, 'text', '')
        if text:
            emit_sse('text_delta', {'text': text})
            return True
    elif delta_type == 'thinking_delta':
        thinking = get_field(delta, 'thinking', '')
        if thinking:
            emit_sse('thinking_delta', {'thinking': thinking})
    elif delta_type == 'input_json_delta':
        partial = get_field(delta, 'partial_json', '') or ''
        tool_input[idx] = tool_input.get(idx, '') + partial
    return False


def process_block_stop(ev, block_types: dict, tool_info: dict, tool_input: dict) -> None:
    idx = get_field(ev, 'index', 0)
    if block_types.get(idx) == 'tool_use' and idx in tool_info:
        raw_input = tool_input.pop(idx, '{}') or '{}'
        try:
            input_data = json.loads(raw_input)
        except json.JSONDecodeError:
            input_data = {}
        info = tool_info.pop(idx)
        emit_sse('tool_start', {'id': info['id'], 'name': info['name'], 'input': input_data})
    block_types.pop(idx, None)


# ── Non-StreamEvent (structured agent events) ─────────────────────────────────

def _class_to_event_type(event) -> str | None:
    """Derive event type from class name for SDKs that don't set a .type attribute.

    e.g. AssistantMessage -> 'assistant', ResultMessage -> 'result'
    """
    name = type(event).__name__
    if name.endswith('Message'):
        return name[:-len('Message')].lower()
    return None


def _block_type(block) -> str | None:
    """Derive content block type from class name when .type attribute is absent.

    e.g. TextBlock -> 'text', ToolUseBlock -> 'tool_use', ThinkingBlock -> 'thinking'
    """
    name = type(block).__name__
    if name == 'TextBlock':
        return 'text'
    if name == 'ToolUseBlock':
        return 'tool_use'
    if name == 'ThinkingBlock':
        return 'thinking'
    return None


def process_agent_event(event, emitted_streaming_text: bool) -> None:
    event_type = get_field(event, 'type') or getattr(event, 'type', None) or _class_to_event_type(event)
    log(f"agent_event  type={event_type!r}  session_id={getattr(event, 'session_id', None)!r}")
    if event_type == 'assistant':
        process_assistant_event(event, emitted_streaming_text)
    elif event_type == 'user':
        process_user_event(event)
    elif event_type == 'result':
        log(f"result  subtype={getattr(event, 'subtype', '?')}")
    elif event_type == 'system':
        log(f"system  subtype={getattr(event, 'subtype', '?')}")


def process_assistant_event(event, emitted_streaming_text: bool) -> None:
    # AssistantMessage exposes .content directly (no .message wrapper)
    content_blocks = getattr(event, 'content', None) or []
    if not content_blocks:
        msg = getattr(event, 'message', None)
        content_blocks = getattr(msg, 'content', []) or [] if msg else []
    block_types = [getattr(b, 'type', type(b).__name__) for b in content_blocks]
    log(f"assistant  blocks={block_types}")
    if emitted_streaming_text:
        # Text already came via streaming deltas; only handle tool_use blocks.
        for block in content_blocks:
            if (getattr(block, 'type', None) or _block_type(block)) == 'tool_use':
                emit_sse('tool_start', {
                    'id': getattr(block, 'id', None),
                    'name': getattr(block, 'name', None),
                    'input': getattr(block, 'input', {}) or {},
                })
        return
    # No streaming text: emit the full message content now.
    for block in content_blocks:
        block_type = getattr(block, 'type', None) or _block_type(block)
        if block_type == 'text':
            text = getattr(block, 'text', '') or ''
            if text:
                emit_sse('init', {})
                emit_sse('text_delta', {'text': text})
        elif block_type == 'thinking':
            thinking = getattr(block, 'thinking', '') or ''
            if thinking:
                emit_sse('thinking_delta', {'thinking': thinking})
        elif block_type == 'tool_use':
            emit_sse('tool_start', {
                'id': getattr(block, 'id', None),
                'name': getattr(block, 'name', None),
                'input': getattr(block, 'input', {}) or {},
            })


def process_user_event(event) -> None:
    msg = getattr(event, 'message', None)
    if not msg:
        return
    tool_ids = []
    for block in (getattr(msg, 'content', []) or []):
        if getattr(block, 'type', None) != 'tool_result':
            continue
        tool_ids.append(getattr(block, 'tool_use_id', None))
        raw_content = getattr(block, 'content', None)
        if isinstance(raw_content, list):
            content_str = ' '.join(
                getattr(b, 'text', '') or ''
                for b in raw_content
                if getattr(b, 'type', None) == 'text'
            )
        else:
            content_str = str(raw_content) if raw_content is not None else ''
        emit_sse('tool_result', {
            'tool_use_id': getattr(block, 'tool_use_id', None),
            'content': content_str,
            'is_error': getattr(block, 'is_error', False) or False,
        })
    log(f"user tool_results  tool_use_ids={tool_ids}")


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


asyncio.run(main())
