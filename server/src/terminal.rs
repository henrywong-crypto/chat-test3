use anyhow::{Context, Result};
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
use ssh_client::{connect_ssh, open_terminal_channel};
use std::time::Duration;
use store::upsert_user;
use tracing::{error, info, warn};
use uuid::Uuid;
use vm_lifecycle::{build_user_rootfs_path, VmEntry};

use crate::{
    auth::User,
    state::{find_vm_guest_ip_for_user, mark_vm_ws_connected, AppError, AppState},
};

pub(crate) async fn handle_ws_upgrade(
    user: User,
    Path(vm_id): Path<String>,
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> Result<Response, AppError> {
    if Uuid::parse_str(&vm_id).is_err() {
        return Ok((StatusCode::NOT_FOUND, "Not found").into_response());
    }
    let db_user = upsert_user(&state.db, &user.email).await?;
    Ok(ws.on_upgrade(move |socket| run_terminal_session(socket, state, vm_id, db_user.id)))
}

async fn run_terminal_session(ws: WebSocket, state: AppState, vm_id: String, user_id: Uuid) {
    let Some(guest_ip) = find_vm_guest_ip_for_user(&state.vms, &vm_id, user_id)
        .inspect_err(|e| error!("vm registry error: {e}"))
        .ok()
        .flatten()
    else {
        return;
    };
    mark_vm_ws_connected(&state.vms, &vm_id)
        .unwrap_or_else(|e| error!("failed to mark VM ws connected: {e}"));
    run_ssh_relay(&guest_ip, &state, ws)
        .await
        .unwrap_or_else(|e| error!("terminal session error: {e}"));
    save_and_drop_vm(&state, &vm_id, user_id).await;
}

async fn save_and_drop_vm(state: &AppState, vm_id: &str, user_id: Uuid) {
    let vm_entry = {
        let Ok(mut registry) = state.vms.lock() else {
            error!("vm registry lock poisoned on disconnect");
            return;
        };
        registry.remove(vm_id)
    };
    let Some(vm_entry) = vm_entry else { return };
    save_vm_rootfs_on_disconnect(state, user_id, vm_entry)
        .await
        .unwrap_or_else(|e| error!("failed to save rootfs on disconnect: {e}"));
}

async fn save_vm_rootfs_on_disconnect(
    state: &AppState,
    user_id: Uuid,
    vm_entry: VmEntry,
) -> Result<()> {
    tokio::fs::create_dir_all(&state.user_rootfs_dir)
        .await
        .context("failed to create user rootfs dir on disconnect")?;
    let user_rootfs = build_user_rootfs_path(&state.user_rootfs_dir, user_id);
    let _guard = state.rootfs_lock.lock().await;
    info!("saving rootfs on disconnect");
    vm_entry
        .vm
        .save_rootfs(&user_rootfs)
        .await
        .context("failed to save rootfs")
}

async fn run_ssh_relay(guest_ip: &str, state: &AppState, ws: WebSocket) -> Result<()> {
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
                        handle_resize_message(&mut ssh_channel, &text).await
                            .unwrap_or_else(|e| warn!("handle_resize_message failed: {e}"));
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

async fn handle_resize_message(ssh_channel: &mut Channel<Msg>, text: &str) -> Result<()> {
    let Ok(json) = serde_json::from_str::<serde_json::Value>(text) else {
        return Ok(());
    };
    if json["type"] == "resize" {
        let cols = json["cols"]
            .as_u64()
            .context("missing cols in resize message")? as u32;
        let rows = json["rows"]
            .as_u64()
            .context("missing rows in resize message")? as u32;
        ssh_channel.window_change(cols, rows, 0, 0).await?;
    }
    Ok(())
}
