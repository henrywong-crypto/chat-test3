use axum::{
    extract::{
        ws::{WebSocket, WebSocketUpgrade},
        Path, State,
    },
    http::StatusCode,
    response::{IntoResponse, Response},
};
use store::upsert_user;
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::{
    auth::User,
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
    ws.on_upgrade(move |socket| run_chat_session(socket, vm_id, guest_ip, state))
}

async fn run_chat_session(socket: WebSocket, vm_id: String, guest_ip: String, state: AppState) {
    if let Err(e) = chat_relay::run_agent_relay(
        &guest_ip,
        &state.ssh_key_path,
        &state.ssh_user,
        &state.vm_host_key_path,
        socket,
        &vm_id,
    )
    .await
    {
        error!("chat session error: {e}");
    }
    info!("chat ws disconnected");
}
