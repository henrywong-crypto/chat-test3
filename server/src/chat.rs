use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Response,
    },
    Json,
};
use chat_relay::{start_agent_relay, AgentMessage};
use serde::Deserialize;
use store::upsert_user;
use tokio::sync::mpsc;
use tower_sessions::Session;
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::{
    auth::User,
    state::{find_vm_guest_ip_for_user, mark_vm_ws_connected, AppState},
};

pub(crate) async fn handle_chat_stream(
    user: User,
    Path(vm_id): Path<String>,
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
            warn!("chat stream: vm not found");
            return (StatusCode::NOT_FOUND, "VM not found").into_response();
        }
    };
    mark_vm_ws_connected(&state.vms, &vm_id);
    let (agent_tx, agent_rx) = mpsc::channel::<AgentMessage>(4);
    state.chat_senders.lock().unwrap().insert(vm_id.clone(), agent_tx);
    let event_stream = match start_agent_relay(
        &guest_ip,
        &state.ssh_key_path,
        &state.ssh_user,
        &state.vm_host_key_path,
        agent_rx,
        vm_id,
    )
    .await
    {
        Ok(event_stream) => event_stream,
        Err(e) => {
            error!("start_agent_relay failed: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Internal error").into_response();
        }
    };
    info!("chat sse stream opened");
    Sse::new(event_stream).keep_alive(KeepAlive::default()).into_response()
}

#[derive(Deserialize)]
pub(crate) struct QueryBody {
    content: String,
    session_id: Option<String>,
    csrf_token: String,
}

pub(crate) async fn handle_chat_query(
    _user: User,
    session: Session,
    Path(vm_id): Path<String>,
    State(state): State<AppState>,
    Json(body): Json<QueryBody>,
) -> Response {
    if Uuid::parse_str(&vm_id).is_err() {
        return (StatusCode::NOT_FOUND, "Not found").into_response();
    }
    if !validate_csrf(&session, &body.csrf_token).await {
        return (StatusCode::FORBIDDEN, "Forbidden").into_response();
    }
    let agent_tx = match find_agent_sender(&state, &vm_id) {
        Some(agent_tx) => agent_tx,
        None => return (StatusCode::NOT_FOUND, "No active chat stream").into_response(),
    };
    let agent_message = AgentMessage::Query { content: body.content, session_id: body.session_id };
    if agent_tx.send(agent_message).await.is_err() {
        return (StatusCode::SERVICE_UNAVAILABLE, "Agent not available").into_response();
    }
    info!(vm_id = %vm_id, "query forwarded");
    (StatusCode::OK, "").into_response()
}

#[derive(Deserialize)]
pub(crate) struct AbortBody {
    csrf_token: String,
}

pub(crate) async fn handle_chat_abort(
    _user: User,
    session: Session,
    Path(vm_id): Path<String>,
    State(state): State<AppState>,
    Json(body): Json<AbortBody>,
) -> Response {
    if Uuid::parse_str(&vm_id).is_err() {
        return (StatusCode::NOT_FOUND, "Not found").into_response();
    }
    if !validate_csrf(&session, &body.csrf_token).await {
        return (StatusCode::FORBIDDEN, "Forbidden").into_response();
    }
    let agent_tx = match find_agent_sender(&state, &vm_id) {
        Some(agent_tx) => agent_tx,
        None => return (StatusCode::NOT_FOUND, "No active chat stream").into_response(),
    };
    let _ = agent_tx.send(AgentMessage::Abort).await;
    info!(vm_id = %vm_id, "abort forwarded");
    (StatusCode::OK, "").into_response()
}

fn find_agent_sender(state: &AppState, vm_id: &str) -> Option<mpsc::Sender<AgentMessage>> {
    state.chat_senders.lock().ok()?.get(vm_id).cloned()
}

async fn validate_csrf(session: &Session, submitted: &str) -> bool {
    match session.get::<String>("csrf_token").await {
        Ok(Some(token)) => token == submitted,
        _ => false,
    }
}
