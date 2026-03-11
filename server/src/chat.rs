use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Path, State,
    },
    http::StatusCode,
    response::{IntoResponse, Response},
};
use bytes::Bytes;
use futures::{SinkExt, StreamExt};
use russh::ChannelMsg;
use store::upsert_user;
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::{
    auth::User,
    ssh::{connect_ssh, open_exec_channel},
    state::{find_vm_guest_ip_for_user, mark_vm_ws_connected, AppState},
};

pub(crate) async fn handle_chat_ws_upgrade(
    user: User,
    Path(vm_id): Path<String>,
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> Response {
    if Uuid::parse_str(&vm_id).is_err() {
        return (StatusCode::NOT_FOUND, "Not found").into_response();
    }
    let db_user = match upsert_user(&state.db, &user.email).await {
        Ok(db_user) => db_user,
        Err(e) => {
            error!(vm_id = %vm_id, "upsert_user failed: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Internal error").into_response();
        }
    };
    let guest_ip = match find_vm_guest_ip_for_user(&state.vms, &vm_id, db_user.id) {
        Some(guest_ip) => guest_ip,
        None => {
            warn!("chat ws upgrade: vm not found");
            return (StatusCode::NOT_FOUND, "VM not found").into_response();
        }
    };
    info!("chat ws connected");
    mark_vm_ws_connected(&state.vms, &vm_id);
    ws.on_upgrade(move |socket| run_chat_session(socket, vm_id, db_user.id.to_string(), guest_ip, state))
}

async fn run_chat_session(socket: WebSocket, vm_id: String, user_id: String, guest_ip: String, state: AppState) {
    if let Err(e) = run_agent_relay(&guest_ip, &state, socket, &vm_id, &user_id).await {
        error!(vm_id = %vm_id, user_id = %user_id, "chat session error: {e}");
    }
    info!("chat ws disconnected");
}

async fn run_agent_relay(
    guest_ip: &str,
    state: &AppState,
    socket: WebSocket,
    vm_id: &str,
    user_id: &str,
) -> anyhow::Result<()> {
    let mut ssh_handle = connect_ssh(
        guest_ip,
        &state.ssh_key_path,
        &state.ssh_user,
        &state.vm_host_key_path,
    )
    .await?;
    info!("agent ssh channel opened");
    let mut ssh_channel = open_exec_channel(&mut ssh_handle, "bash -lc '/usr/local/bin/uv run /opt/agent.py 2> >(tee -a /home/ubuntu/agent.log >&2)'").await?;
    let (mut ws_sender, mut ws_receiver) = socket.split();
    let mut line_buf = String::new();
    loop {
        tokio::select! {
            msg = ssh_channel.wait() => {
                match msg {
                    Some(ChannelMsg::Data { ref data }) => {
                        let chunk = std::str::from_utf8(data).unwrap_or("");
                        line_buf.push_str(chunk);
                        while let Some(newline_pos) = line_buf.find('\n') {
                            let line = line_buf[..newline_pos].trim_end_matches('\r').to_owned();
                            line_buf.drain(..=newline_pos);
                            let line = normalize_event_line(line);
                            log_agent_event(vm_id, &line);
                            if ws_sender.send(Message::Text(line.into())).await.is_err() {
                                return Ok(());
                            }
                        }
                    }
                    Some(ChannelMsg::ExtendedData { ref data, .. }) => {
                        // agent.py stderr lands here — forward to server logs
                        if let Ok(text) = std::str::from_utf8(data) {
                            for stderr_line in text.lines() {
                                if !stderr_line.is_empty() {
                                    info!(vm_id = %vm_id, "{stderr_line}");
                                }
                            }
                        }
                    }
                    Some(ChannelMsg::ExitStatus { exit_status }) => {
                        info!("agent exited  status={exit_status}");
                        break;
                    }
                    None => break,
                    _ => {}
                }
            }
            ws_msg = ws_receiver.next() => {
                match ws_msg {
                    Some(Ok(Message::Text(text))) => {
                        if is_abort_message(text.as_str()) {
                            info!("abort received, closing agent channel");
                            break;
                        }
                        log_query(vm_id, text.as_str());
                        let line = format!("{}\n", text.as_str());
                        if ssh_channel.data(Bytes::from(line).as_ref()).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Ping(data))) => {
                        let _ = ws_sender.send(Message::Pong(data)).await;
                    }
                    Some(Ok(Message::Pong(_))) => {}
                    _ => break,
                }
            }
        }
    }
    Ok(())
}

/// Returns true if the WebSocket message is an abort request.
fn is_abort_message(text: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(text)
        .map(|v| v["type"].as_str() == Some("abort"))
        .unwrap_or(false)
}

/// Re-injects the `type` field into agent events when Pydantic's model_dump() omitted it.
///
/// The Claude Agent SDK uses Pydantic v2 discriminated unions whose `type` field is omitted
/// by `model_dump()`. agent.py attempts to re-inject it, but older deployed versions may not.
/// This function acts as a server-side safety net: if a valid JSON object arrives without
/// a `type` field, we infer the type from other structural fields.
fn normalize_event_line(line: String) -> String {
    let Ok(mut v) = serde_json::from_str::<serde_json::Value>(&line) else {
        return line;
    };
    let obj = match v.as_object_mut() {
        Some(obj) => obj,
        None => return line,
    };
    if obj.get("type").and_then(|t| t.as_str()) == Some("stream_event") {
        let changed = normalize_stream_event_inner(obj);
        return if changed {
            serde_json::to_string(obj).unwrap_or(line)
        } else {
            line
        };
    }
    let mut changed = false;
    if !obj.contains_key("type") || obj["type"] == serde_json::Value::Null {
        if let Some(t) = infer_event_type(obj) {
            obj.insert("type".to_owned(), serde_json::Value::String(t.to_owned()));
            changed = true;
        }
    }
    // Content blocks from Pydantic model_dump() also lose their `type` discriminator.
    // Inject it for assistant and user events so the frontend can render them.
    let event_type = obj.get("type").and_then(|t| t.as_str()).unwrap_or("");
    if matches!(event_type, "assistant" | "user") {
        changed |= normalize_event_content_blocks(obj);
    }
    if changed {
        serde_json::to_string(obj).unwrap_or(line)
    } else {
        line
    }
}

fn normalize_event_content_blocks(obj: &mut serde_json::Map<String, serde_json::Value>) -> bool {
    if let Some(serde_json::Value::Array(blocks)) = obj.get_mut("content") {
        return normalize_blocks(blocks);
    }
    if let Some(message) = obj.get_mut("message").and_then(|m| m.as_object_mut()) {
        if let Some(serde_json::Value::Array(blocks)) = message.get_mut("content") {
            return normalize_blocks(blocks);
        }
    }
    false
}

fn normalize_blocks(blocks: &mut Vec<serde_json::Value>) -> bool {
    let mut changed = false;
    for block in blocks.iter_mut() {
        let Some(block_obj) = block.as_object_mut() else { continue };
        if block_obj.contains_key("type") {
            continue;
        }
        if let Some(t) = infer_block_type(block_obj) {
            block_obj.insert("type".to_owned(), serde_json::Value::String(t.to_owned()));
            changed = true;
        }
    }
    changed
}

fn infer_block_type(block: &serde_json::Map<String, serde_json::Value>) -> Option<&'static str> {
    if block.contains_key("text") {
        return Some("text");
    }
    if block.contains_key("thinking") {
        return Some("thinking");
    }
    if block.contains_key("id") && block.contains_key("name") && block.contains_key("input") {
        return Some("tool_use");
    }
    if block.contains_key("tool_use_id") {
        return Some("tool_result");
    }
    None
}

/// Normalizes the nested `event` object inside a `stream_event` line.
/// Returns true if the inner event was modified.
fn normalize_stream_event_inner(obj: &mut serde_json::Map<String, serde_json::Value>) -> bool {
    let inner = match obj.get_mut("event").and_then(|e| e.as_object_mut()) {
        Some(inner) => inner,
        None => return false,
    };
    if inner.contains_key("type") && inner["type"] != serde_json::Value::Null {
        // Also normalize delta.type inside content_block_delta events
        if inner.get("type").and_then(|t| t.as_str()) == Some("content_block_delta") {
            return normalize_delta_type(inner);
        }
        return false;
    }
    let inferred = infer_stream_inner_type(inner);
    if let Some(t) = inferred {
        inner.insert("type".to_owned(), serde_json::Value::String(t.to_owned()));
        // Also normalize delta.type for content_block_delta
        if t == "content_block_delta" {
            normalize_delta_type(inner);
        }
        true
    } else {
        false
    }
}

fn normalize_delta_type(inner: &mut serde_json::Map<String, serde_json::Value>) -> bool {
    let delta = match inner.get_mut("delta").and_then(|d| d.as_object_mut()) {
        Some(d) => d,
        None => return false,
    };
    if delta.contains_key("type") && delta["type"] != serde_json::Value::Null {
        return false;
    }
    let inferred = if delta.contains_key("text") {
        Some("text_delta")
    } else if delta.contains_key("thinking") {
        Some("thinking_delta")
    } else if delta.contains_key("partial_json") {
        Some("input_json_delta")
    } else {
        None
    };
    if let Some(t) = inferred {
        delta.insert("type".to_owned(), serde_json::Value::String(t.to_owned()));
        true
    } else {
        false
    }
}

fn infer_stream_inner_type(inner: &serde_json::Map<String, serde_json::Value>) -> Option<&'static str> {
    if inner.contains_key("delta") {
        return Some("content_block_delta");
    }
    if inner.contains_key("content_block") {
        return Some("content_block_start");
    }
    if inner.contains_key("message") {
        return Some("message_start");
    }
    None
}

fn infer_event_type(obj: &serde_json::Map<String, serde_json::Value>) -> Option<&'static str> {
    let subtype = obj.get("subtype").and_then(|v| v.as_str()).unwrap_or("");
    if matches!(subtype, "init" | "cwd") {
        return Some("system");
    }
    if obj.contains_key("subtype") && matches!(subtype, "success" | "error_during_execution") {
        return Some("result");
    }
    // Wrapped SDK format: message.role
    match obj.get("message").and_then(|m| m.get("role")).and_then(|r| r.as_str()) {
        Some("assistant") => return Some("assistant"),
        Some("user") => return Some("user"),
        _ => {}
    }
    // Unwrapped format: content array at top level (agent.py strips the message wrapper)
    if let Some(first) = obj.get("content").and_then(|c| c.as_array()).and_then(|a| a.first()) {
        if first.get("tool_use_id").is_some() {
            return Some("user");
        }
        if first.get("text").is_some()
            || first.get("thinking").is_some()
            || (first.get("id").is_some() && first.get("name").is_some())
        {
            return Some("assistant");
        }
    }
    None
}

fn log_query(vm_id: &str, text: &str) {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(text) else { return };
    match v["type"].as_str().unwrap_or("") {
        "query" => {
            let session = if v["session_id"].as_str().unwrap_or("") == "" { "new" } else { "resume" };
            info!(vm_id = %short_id(vm_id), "query  {session}");
        }
        other => info!(vm_id = %short_id(vm_id), "query  type={other}"),
    }
}

fn log_agent_event(vm_id: &str, line: &str) {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { return };
    let vm = short_id(vm_id);
    match v["type"].as_str().unwrap_or("") {
        "stream_event" => log_stream_event(vm, &v),
        "assistant" => log_assistant_event(vm, &v),
        "user" => log_user_event(vm, &v),
        "system" => {
            let subtype = v["subtype"].as_str().unwrap_or("?");
            info!(vm_id = %vm, "system  {subtype}");
        }
        "result" => {
            let subtype = v["subtype"].as_str().unwrap_or("?");
            if subtype == "success" {
                info!(vm_id = %vm, "result  success");
            } else {
                warn!(vm_id = %vm, "result  {subtype}");
            }
        }
        "done" => info!(vm_id = %vm, "done"),
        "error" => warn!(vm_id = %vm, "error"),
        _ => {}
    }
}

fn log_stream_event(vm: &str, v: &serde_json::Value) {
    let inner = &v["event"];
    match inner["type"].as_str().unwrap_or("") {
        "content_block_delta" | "message_delta" => {}
        "content_block_start" => {
            let block_type = inner["content_block"]["type"].as_str().unwrap_or("?");
            info!(vm_id = %vm, "stream  block_start  {block_type}");
        }
        other => info!(vm_id = %vm, "stream  {other}"),
    }
}

fn log_assistant_event(vm: &str, v: &serde_json::Value) {
    let blocks = v["message"]["content"].as_array()
        .or_else(|| v["content"].as_array());
    let Some(blocks) = blocks else { return };
    for block in blocks {
        let block_type = block["type"].as_str().unwrap_or("");
        if block_type == "text" || block.get("text").is_some() {
            info!(vm_id = %vm, "assistant  text");
        } else if block_type == "tool_use" || block.get("name").is_some() {
            let name = block["name"].as_str().unwrap_or("?");
            info!(vm_id = %vm, "assistant  tool_use  {name}");
        }
    }
}

fn log_user_event(vm: &str, v: &serde_json::Value) {
    let blocks = v["message"]["content"].as_array()
        .or_else(|| v["content"].as_array());
    let Some(blocks) = blocks else { return };
    for block in blocks {
        if block["type"].as_str() == Some("tool_result") || block.get("tool_use_id").is_some() {
            let is_error = block["is_error"].as_bool().unwrap_or(false);
            let status = if is_error { "error" } else { "ok" };
            info!(vm_id = %vm, "user  tool_result  {status}");
        }
    }
}

fn short_id(id: &str) -> &str {
    match id.char_indices().nth(8) {
        Some((i, _)) => &id[..i],
        None => id,
    }
}
