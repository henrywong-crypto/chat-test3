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
    ws.on_upgrade(move |socket| run_chat_session(socket, guest_ip, state))
}

async fn run_chat_session(socket: WebSocket, guest_ip: String, state: AppState) {
    if let Err(e) = run_agent_relay(&guest_ip, &state, socket).await {
        error!("chat session error: {e}");
    }
}

async fn run_agent_relay(
    guest_ip: &str,
    state: &AppState,
    socket: WebSocket,
) -> anyhow::Result<()> {
    let mut ssh_handle = connect_ssh(
        guest_ip,
        &state.ssh_key_path,
        &state.ssh_user,
        &state.vm_host_key_path,
    )
    .await?;
    // bash -l sources ~/.profile → ~/.bashrc so nvm's node and the claude binary are on PATH.
    let mut ssh_channel = open_exec_channel(&mut ssh_handle, "bash -lc 'uv run /opt/agent.py'").await?;
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
