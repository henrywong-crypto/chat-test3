import asyncio
import contextvars
import dataclasses
import json
import os
import sys

SOCKET_PATH = "/tmp/agent.sock"
QUESTION_TIMEOUT_SECS = 3600


def log(msg: str) -> None:
    """Write a log line to stderr so it appears in server logs without polluting the stdout protocol."""
    sys.stderr.write(f"[agent] {msg}\n")
    sys.stderr.flush()


def emit_sse(event_name: str, data: dict) -> None:
    """Write a properly formatted SSE event to the query-scoped writer."""
    writer = _emit_writer.get()
    if writer is None or writer.is_closing():
        return
    payload = f"event: {event_name}\ndata: {json.dumps(data, cls=_Encoder)}\n\n"
    writer.write(payload.encode())


def get_field(obj, field, default=None):
    """Get a field from either a dict or an object attribute."""
    if isinstance(obj, dict):
        return obj.get(field, default)
    return getattr(obj, field, default)


# Pending AskUserQuestion future (at most one active at a time).
_pending_question: asyncio.Future | None = None
# Data for the pending question — re-emitted to reconnecting clients.
_pending_question_data: dict | None = None
# Queue for incoming messages (queries, etc.) from connected clients.
_stdin_queue: asyncio.Queue = asyncio.Queue()
# The currently running query task, so it can be cancelled on interrupt.
_current_query_task: asyncio.Task | None = None
# The writer that submitted the currently running query — only that connection may interrupt it.
_current_query_writer: asyncio.StreamWriter | None = None
# Tracks the most recently active writer for reconnect detection.
_current_writer: asyncio.StreamWriter | None = None
# Per-task writer context: set before spawning each query task so emit_sse always
# routes to the connection that submitted the query, even if _current_writer changes
# while the query is running.
_emit_writer: contextvars.ContextVar[asyncio.StreamWriter | None] = contextvars.ContextVar(
    'emit_writer', default=None
)


async def route_connection(reader: asyncio.StreamReader, writer: asyncio.StreamWriter) -> None:
    """Read lines from a connected client and route them; return on EOF without exiting."""
    global _current_writer
    while True:
        raw = await reader.readline()
        if not raw:
            return
        line = raw.decode().strip()
        if not line:
            continue
        try:
            msg = json.loads(line)
        except json.JSONDecodeError:
            log(f"failed to parse line: {line[:120]}")
            continue
        if msg.get('type') == 'answer_question':
            request_id = msg.get('request_id')
            if (
                _pending_question and not _pending_question.done()
                and _pending_question_data
                and _pending_question_data.get('request_id') == request_id
            ):
                _pending_question.set_result(msg.get('answers', {}))
        elif msg.get('type') == 'interrupt':
            if (
                _current_query_task and not _current_query_task.done()
                and writer is _current_query_writer
            ):
                log("interrupt received, cancelling query task")
                _current_query_task.cancel()
        else:
            _current_writer = writer
            await _stdin_queue.put((msg, writer))


async def handle_connection(reader: asyncio.StreamReader, writer: asyncio.StreamWriter) -> None:
    """Handle a single client connection."""
    global _current_writer
    log("client connected")
    previous_writer = _current_writer
    previous_dropped = previous_writer is None or previous_writer.is_closing()
    if previous_dropped:
        _current_writer = writer
        if _pending_question_data:
            log("re-emitting pending question to reconnected client")
            token = _emit_writer.set(writer)
            emit_sse('ask_user_question', _pending_question_data)
            _emit_writer.reset(token)
    await route_connection(reader, writer)
    if _current_writer is writer:
        _current_writer = None
    writer.close()
    await writer.wait_closed()
    log("client disconnected")


async def process_query_queue() -> None:
    """Consume _stdin_queue and run queries sequentially."""
    global _current_query_task, _current_query_writer
    while True:
        msg, query_writer = await _stdin_queue.get()
        if msg.get('type') == 'query':
            _current_query_writer = query_writer
            token = _emit_writer.set(query_writer)
            _current_query_task = asyncio.create_task(
                run_query(msg.get('content', ''), msg.get('session_id'))
            )
            _emit_writer.reset(token)
            try:
                await _current_query_task
            except asyncio.CancelledError:
                pass
            finally:
                _current_query_writer = None


async def main():
    try:
        os.unlink(SOCKET_PATH)
    except FileNotFoundError:
        pass
    server = await asyncio.start_unix_server(handle_connection, path=SOCKET_PATH)
    log("agent daemon ready")
    asyncio.create_task(process_query_queue())
    async with server:
        await server.serve_forever()


async def run_query(content: str, session_id):
    from claude_agent_sdk import ClaudeAgentOptions, PermissionResultAllow, query
    from claude_agent_sdk.types import HookMatcher, StreamEvent

    log(f"query start  session_id={session_id!r}  content_len={len(content)}")

    async def handle_tool_permission(tool_name, input_, context):
        log(f"can_use_tool called  tool_name={tool_name!r}")
        return PermissionResultAllow()

    async def ask_user_question_hook(input_data, tool_use_id, context):
        global _pending_question, _pending_question_data
        tool_input = get_field(input_data, 'tool_input') or {}
        questions = tool_input.get('questions', []) if isinstance(tool_input, dict) else []
        _pending_question = asyncio.get_running_loop().create_future()
        _pending_question_data = {'request_id': tool_use_id, 'session_id': captured_session_id, 'questions': questions}
        emit_sse('ask_user_question', {'request_id': tool_use_id, 'session_id': captured_session_id, 'questions': questions})
        log(f"PreToolUse AskUserQuestion: waiting for answer")
        answers = await _pending_question
        _pending_question = None
        _pending_question_data = None
        log(f"PreToolUse AskUserQuestion: answered")
        return {
            'hookSpecificOutput': {
                'hookEventName': 'PreToolUse',
                'updatedInput': {**tool_input, 'answers': answers},
            }
        }

    options = ClaudeAgentOptions(
        cwd=os.environ.get('HOME', '/root'),
        can_use_tool=handle_tool_permission,
        hooks={
            'PreToolUse': [HookMatcher(matcher='AskUserQuestion', hooks=[ask_user_question_hook], timeout=QUESTION_TIMEOUT_SECS)],
        },
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
    except asyncio.CancelledError:
        log("query cancelled by interrupt")
    except Exception as exc:
        log(f"query error: {exc}")
        emit_sse('error_event', {'message': str(exc)})
    finally:
        global _pending_question, _pending_question_data
        _pending_question = None
        _pending_question_data = None
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
        if info['name'] != 'AskUserQuestion':
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
                if (getattr(block, 'name', None) or '') != 'AskUserQuestion':
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
            block_name = getattr(block, 'name', None) or ''
            if block_name != 'AskUserQuestion':
                emit_sse('tool_start', {
                    'id': getattr(block, 'id', None),
                    'name': block_name,
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
