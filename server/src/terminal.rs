use bytes::Bytes;
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Path, State,
    },
    response::Response,
};
use futures::{SinkExt, StreamExt};
use russh::ChannelMsg;

use crate::{
    ssh::{connect_ssh, open_terminal_channel},
    state::{AppState, find_vm_guest_ip},
};

pub(crate) async fn handle_ws_upgrade(
    Path(vm_id): Path<String>,
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> Response {
    ws.on_upgrade(|socket| run_terminal_session(socket, state, vm_id))
}

async fn run_terminal_session(ws: WebSocket, state: AppState, vm_id: String) {
    let guest_ip = match find_vm_guest_ip(&state.vms, &vm_id) {
        Some(ip) => ip,
        None => {
            eprintln!("VM {vm_id} not found");
            return;
        }
    };
    if let Err(e) = run_ssh_relay(&guest_ip, &state, ws).await {
        eprintln!("SSH session error [{vm_id}]: {e}");
    }
}

async fn run_ssh_relay(guest_ip: &str, state: &AppState, ws: WebSocket) -> anyhow::Result<()> {
    let mut ssh_handle =
        connect_ssh(guest_ip, &state.ssh_key_path, &state.ssh_user, &state.vm_host_key).await?;
    let mut channel = open_terminal_channel(&mut ssh_handle).await?;
    let (mut ws_sender, mut ws_receiver) = ws.split();
    loop {
        tokio::select! {
            msg = channel.wait() => {
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
                        if channel.data(&data[..]).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Text(text))) => {
                        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                            if json["type"] == "resize" {
                                let cols = json["cols"].as_u64().unwrap_or(80) as u32;
                                let rows = json["rows"].as_u64().unwrap_or(24) as u32;
                                let _ = channel.window_change(cols, rows, 0, 0).await;
                            }
                        }
                    }
                    _ => break,
                }
            }
        }
    }
    Ok(())
}
