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
    let mut ssh_channel = open_exec_channel(&mut ssh_handle, "bash -lc '/usr/local/bin/uv run /opt/agent.py'").await?;
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
        _ => {}
    }
}
