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
    state::{find_vm_guest_ip_for_user, AppState},
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
            warn!(vm_id = %vm_id, user_id = %db_user.id, "chat ws upgrade: vm not found for user");
            return (StatusCode::NOT_FOUND, "VM not found").into_response();
        }
    };
    info!(vm_id = %vm_id, user_id = %db_user.id, "chat ws connected");
    ws.on_upgrade(move |socket| run_chat_session(socket, vm_id, db_user.id.to_string(), guest_ip, state))
}

async fn run_chat_session(socket: WebSocket, vm_id: String, user_id: String, guest_ip: String, state: AppState) {
    if let Err(e) = run_agent_relay(&guest_ip, &state, socket, &vm_id, &user_id).await {
        error!(vm_id = %vm_id, user_id = %user_id, "chat session error: {e}");
    }
    info!(vm_id = %vm_id, user_id = %user_id, "chat ws disconnected");
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
    info!(vm_id = %vm_id, user_id = %user_id, "agent ssh channel opened");
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
                        info!(vm_id = %vm_id, user_id = %user_id, "agent exited  status={exit_status}");
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
                            info!(vm_id = %vm_id, user_id = %user_id, "abort received, closing agent channel");
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
    // For stream_event: outer type is hardcoded by agent.py, but the inner
    // event object's type field may be stripped by Pydantic model_dump().
    if obj.get("type").and_then(|t| t.as_str()) == Some("stream_event") {
        let changed = normalize_stream_event_inner(obj);
        return if changed {
            serde_json::to_string(obj).unwrap_or(line)
        } else {
            line
        };
    }
    if obj.contains_key("type") && obj["type"] != serde_json::Value::Null {
        return line;
    }
    let inferred_type = infer_event_type(obj);
    if let Some(t) = inferred_type {
        obj.insert("type".to_owned(), serde_json::Value::String(t.to_owned()));
        serde_json::to_string(obj).unwrap_or(line)
    } else {
        line
    }
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
    if matches!(subtype, "success" | "error_during_execution") || obj.contains_key("session_id") {
        // result events always carry subtype + session_id; guard against plain dicts
        if obj.contains_key("subtype") {
            return Some("result");
        }
    }
    let role = obj.get("message")
        .and_then(|m| m.get("role"))
        .and_then(|r| r.as_str())
        .unwrap_or("");
    match role {
        "assistant" => Some("assistant"),
        "user" => Some("user"),
        _ => None,
    }
}

/// Log a query message received from the browser WebSocket.
fn log_query(vm_id: &str, text: &str) {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(text) else { return };
    let msg_type = v["type"].as_str().unwrap_or("?");
    if msg_type == "query" {
        let content_preview: String = v["content"].as_str().unwrap_or("").chars().take(80).collect();
        let session_id = v["session_id"].as_str().unwrap_or("null");
        info!(vm_id = %vm_id, "ws→agent  type=query  session_id={session_id}  content={content_preview:?}");
    } else {
        info!(vm_id = %vm_id, "ws→agent  type={msg_type}");
    }
}

/// Log a significant event line emitted by the agent before forwarding to the browser.
fn log_agent_event(vm_id: &str, line: &str) {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { return };
    let event_type = v["type"].as_str().unwrap_or("?");
    match event_type {
        // stream_event: only log structural ones, skip noisy deltas
        "stream_event" => {
            let inner_type = v["event"]["type"].as_str().unwrap_or("?");
            if !matches!(inner_type, "content_block_delta" | "message_delta") {
                info!(vm_id = %vm_id, "agent→ws  stream_event  {inner_type}");
            }
        }
        "assistant" => {
            let blocks: Vec<&str> = v["message"]["content"]
                .as_array()
                .map(|arr| arr.iter().filter_map(|b| b["type"].as_str()).collect())
                .unwrap_or_default();
            info!(vm_id = %vm_id, "agent→ws  assistant  blocks={blocks:?}");
        }
        "user" => {
            let tool_ids: Vec<&str> = v["message"]["content"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter(|b| b["type"].as_str() == Some("tool_result"))
                        .filter_map(|b| b["tool_use_id"].as_str())
                        .collect()
                })
                .unwrap_or_default();
            info!(vm_id = %vm_id, "agent→ws  user  tool_result_ids={tool_ids:?}");
        }
        "result" => {
            let subtype = v["subtype"].as_str().unwrap_or("?");
            let session_id = v["session_id"].as_str().unwrap_or("null");
            info!(vm_id = %vm_id, "agent→ws  result  subtype={subtype}  session_id={session_id}");
        }
        "done" => {
            let session_id = v["session_id"].as_str().unwrap_or("null");
            info!(vm_id = %vm_id, "agent→ws  done  session_id={session_id}");
        }
        "error" => {
            let message = v["message"].as_str().unwrap_or("?");
            warn!(vm_id = %vm_id, "agent→ws  error  message={message:?}");
        }
        "system" => {
            let subtype = v["subtype"].as_str().unwrap_or("?");
            info!(vm_id = %vm_id, "agent→ws  system  subtype={subtype}");
        }
        _ => {
            // Log a preview of unrecognised events so they don't disappear silently.
            let preview: String = line.chars().take(200).collect();
            warn!(vm_id = %vm_id, "agent→ws  unknown  {preview}");
        }
    }
}
