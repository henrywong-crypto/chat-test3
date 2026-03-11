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
use store::{upsert_chat_session, upsert_user};
use tracing::error;
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
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "Internal error").into_response(),
    };
    let guest_ip = match find_vm_guest_ip_for_user(&state.vms, &vm_id, db_user.id) {
        Some(guest_ip) => guest_ip,
        None => return (StatusCode::NOT_FOUND, "VM not found").into_response(),
    };
    let user_id = db_user.id;
    ws.on_upgrade(move |socket| run_chat_session(socket, guest_ip, vm_id, user_id, state))
}

async fn run_chat_session(
    socket: WebSocket,
    guest_ip: String,
    vm_id: String,
    user_id: Uuid,
    state: AppState,
) {
    if let Err(e) = run_agent_relay(&guest_ip, &state, socket, user_id, &vm_id).await {
        error!("chat session error: {e}");
    }
}

async fn run_agent_relay(
    guest_ip: &str,
    state: &AppState,
    socket: WebSocket,
    user_id: Uuid,
    vm_id: &str,
) -> anyhow::Result<()> {
    let mut ssh_handle = connect_ssh(
        guest_ip,
        &state.ssh_key_path,
        &state.ssh_user,
        &state.vm_host_key_path,
    )
    .await?;
    // bash -l sources ~/.profile → ~/.bashrc so the claude binary installed by
    // https://claude.ai/install.sh is on PATH. uv is addressed by full path since
    // /usr/local/bin may not be in the login shell's PATH on the minimal image.
    let mut ssh_channel = open_exec_channel(&mut ssh_handle, "bash -lc '/usr/local/bin/uv run /opt/agent.py'").await?;
    let (mut ws_sender, mut ws_receiver) = socket.split();
    let mut line_buf = String::new();
    let mut pending_title = String::new();
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
                            persist_session_if_done(&line, state, user_id, vm_id, &pending_title);
                            if ws_sender.send(Message::Text(line.into())).await.is_err() {
                                return Ok(());
                            }
                        }
                    }
                    Some(ChannelMsg::ExitStatus { .. }) | None => break,
                    _ => {}
                }
            }
            ws_msg = ws_receiver.next() => {
                match ws_msg {
                    Some(Ok(Message::Text(text))) => {
                        if pending_title.is_empty() {
                            capture_pending_title(text.as_str(), &mut pending_title);
                        }
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

fn capture_pending_title(text: &str, pending_title: &mut String) {
    let Ok(json_value) = serde_json::from_str::<serde_json::Value>(text) else { return };
    if json_value.get("type").and_then(|t| t.as_str()) != Some("query") { return };
    let Some(content) = json_value.get("content").and_then(|c| c.as_str()) else { return };
    *pending_title = content.chars().take(60).collect();
}

fn persist_session_if_done(line: &str, state: &AppState, user_id: Uuid, vm_id: &str, pending_title: &str) {
    let Ok(json_value) = serde_json::from_str::<serde_json::Value>(line) else { return };
    if json_value.get("type").and_then(|t| t.as_str()) != Some("done") { return };
    let Some(session_id) = json_value.get("session_id").and_then(|s| s.as_str()) else { return };
    let title = if pending_title.is_empty() { session_id.to_owned() } else { pending_title.to_owned() };
    let db = state.db.clone();
    let session_id = session_id.to_owned();
    let vm_id = vm_id.to_owned();
    tokio::spawn(async move {
        let _ = upsert_chat_session(&db, user_id, &vm_id, &session_id, &title).await;
    });
}
