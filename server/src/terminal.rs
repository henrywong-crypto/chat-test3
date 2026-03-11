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
use std::time::Duration;
use store::upsert_user;
use ssh_client::{connect_ssh, open_terminal_channel};
use tracing::{error, info};
use uuid::Uuid;

use crate::{
    auth::User,
    state::{find_vm_guest_ip_for_user, mark_vm_ws_connected, AppState},
    vm::build_user_rootfs_path,
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
    let db_user = match upsert_user(&state.db, &user.email).await {
        Ok(db_user) => db_user,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "Internal error").into_response(),
    };
    ws.on_upgrade(move |socket| run_terminal_session(socket, state, vm_id, db_user.id))
}

async fn run_terminal_session(ws: WebSocket, state: AppState, vm_id: String, user_id: Uuid) {
    let guest_ip = match find_vm_guest_ip_for_user(&state.vms, &vm_id, user_id) {
        Some(guest_ip) => guest_ip,
        None => return,
    };
    mark_vm_ws_connected(&state.vms, &vm_id);
    if let Err(e) = run_ssh_relay(&guest_ip, &state, ws).await {
        error!("terminal session error: {e}");
    }
    save_and_drop_vm(&state, &vm_id, user_id).await;
}

async fn save_and_drop_vm(state: &AppState, vm_id: &str, user_id: Uuid) {
    let vm_entry = {
        let Ok(mut registry) = state.vms.lock() else {
            return;
        };
        registry.remove(vm_id)
    };
    let Some(vm_entry) = vm_entry else {
        return;
    };
    if let Err(e) = tokio::fs::create_dir_all(&state.user_rootfs_dir).await {
        error!(vm_id = %vm_id, "failed to create user rootfs dir on disconnect: {e}");
        return;
    }
    let user_rootfs = build_user_rootfs_path(&state.user_rootfs_dir, user_id);
    let _guard = state.rootfs_lock.lock().await;
    info!("saving rootfs on disconnect");
    if let Err(e) = vm_entry.vm.save_rootfs(&user_rootfs).await {
        error!(vm_id = %vm_id, "failed to save rootfs on disconnect: {e}");
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
    let mut keepalive = tokio::time::interval(Duration::from_secs(30));
    keepalive.tick().await; // skip the immediate first tick
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
                    Some(Ok(Message::Ping(data))) => {
                        let _ = ws_sender.send(Message::Pong(data)).await;
                    }
                    Some(Ok(Message::Pong(_))) => {}
                    _ => break,
                }
            }
            _ = keepalive.tick() => {
                if ws_sender.send(Message::Ping(Bytes::new())).await.is_err() {
                    break;
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
