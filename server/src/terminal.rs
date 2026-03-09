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
use russh::{client::Msg, Channel, ChannelMsg};
use tracing::error;
use uuid::Uuid;

use crate::{
    auth::User,
    ssh::{connect_ssh, open_terminal_channel},
    state::{find_vm_guest_ip_for_user, AppState},
};

pub(crate) async fn handle_ws_upgrade(
    user: User,
    Path(vm_id): Path<String>,
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> Response {
    if Uuid::parse_str(&vm_id).is_err() {
        return (StatusCode::NOT_FOUND, "Not found").into_response();
    }
    let email = user.email;
    ws.on_upgrade(move |socket| run_terminal_session(socket, state, vm_id, email))
}

async fn run_terminal_session(ws: WebSocket, state: AppState, vm_id: String, email: String) {
    let guest_ip = match find_vm_guest_ip_for_user(&state.vms, &vm_id, &email) {
        Some(ip) => ip,
        None => return,
    };
    if let Err(e) = run_ssh_relay(&guest_ip, &state, ws).await {
        error!("terminal session error: {e}");
    }
}

async fn run_ssh_relay(guest_ip: &str, state: &AppState, ws: WebSocket) -> anyhow::Result<()> {
    let mut ssh_handle = connect_ssh(
        guest_ip,
        &state.ssh_key_path,
        &state.ssh_user,
        &state.vm_host_key_path,
    )
    .await?;
    let mut ssh_channel = open_terminal_channel(&mut ssh_handle).await?;
    let (mut ws_sender, mut ws_receiver) = ws.split();
    loop {
        tokio::select! {
            msg = ssh_channel.wait() => {
                match msg {
                    Some(ChannelMsg::Data { ref data }) => {
                        if ws_sender.send(Message::Binary(Bytes::copy_from_slice(data))).await.is_err() {
                            break;
                        }
                    }
                    Some(ChannelMsg::ExitStatus { .. }) | None => break,
                    _ => {}
                }
            }
            ws_msg = ws_receiver.next() => {
                match ws_msg {
                    Some(Ok(Message::Binary(data))) => {
                        if ssh_channel.data(&data[..]).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Text(text))) => {
                        handle_resize_message(&mut ssh_channel, &text).await;
                    }
                    _ => break,
                }
            }
        }
    }
    Ok(())
}

async fn handle_resize_message(ssh_channel: &mut Channel<Msg>, text: &str) {
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(text) {
        if json["type"] == "resize" {
            let cols = json["cols"].as_u64().unwrap_or(80) as u32;
            let rows = json["rows"].as_u64().unwrap_or(24) as u32;
            let _ = ssh_channel.window_change(cols, rows, 0, 0).await;
        }
    }
}
