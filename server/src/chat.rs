use anyhow::anyhow;
use axum::{
    body::Body,
    extract::{Path, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use chat_relay::{start_agent_relay, AgentMessage};
use futures::StreamExt;
use serde::Deserialize;
use store::upsert_user;
use tokio::sync::mpsc;
use tower_sessions::Session;
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::{
    auth::User,
    state::{find_vm_guest_ip_for_user, mark_vm_ws_connected, AppError, AppState},
};

pub(crate) async fn handle_chat_stream(
    user: User,
    Path(vm_id): Path<String>,
    State(state): State<AppState>,
) -> Result<Response, AppError> {
    if Uuid::parse_str(&vm_id).is_err() {
        return Ok((StatusCode::NOT_FOUND, "Not found").into_response());
    }
    let db_user = upsert_user(&state.db, &user.email).await?;
    let Some(guest_ip) = find_vm_guest_ip_for_user(&state.vms, &vm_id, db_user.id)? else {
        return Ok((StatusCode::NOT_FOUND, "VM not found").into_response());
    };
    mark_vm_ws_connected(&state.vms, &vm_id)
        .unwrap_or_else(|e| error!("failed to mark VM ws connected: {e}"));
    let (agent_tx, agent_rx) = mpsc::channel::<AgentMessage>(4);
    state.chat_senders
        .lock()
        .map_err(|e| anyhow!("chat senders lock poisoned: {e}"))?
        .insert(vm_id.clone(), agent_tx);
    let event_stream = start_agent_relay(
        &guest_ip,
        &state.ssh_key_path,
        &state.ssh_user,
        &state.vm_host_key_path,
        agent_rx,
    )
    .await?;
    info!("chat sse stream opened");
    let body = Body::from_stream(event_stream.map(Ok::<_, std::convert::Infallible>));
    Response::builder()
        .header(header::CONTENT_TYPE, "text/event-stream")
        .header(header::CACHE_CONTROL, "no-cache")
        .body(body)
        .map_err(|e| anyhow!("failed to build SSE response: {e}"))
        .map_err(AppError::from)
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
    let Some(agent_tx) = find_agent_sender(&state, &vm_id) else {
        return (StatusCode::NOT_FOUND, "No active chat stream").into_response();
    };
    let agent_message = AgentMessage::Query {
        content: body.content,
        session_id: body.session_id,
    };
    if agent_tx.send(agent_message).await.is_err() {
        return (StatusCode::SERVICE_UNAVAILABLE, "Agent not available").into_response();
    }
    info!("query forwarded");
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
    let Some(agent_tx) = find_agent_sender(&state, &vm_id) else {
        return (StatusCode::NOT_FOUND, "No active chat stream").into_response();
    };
    let _ = agent_tx.send(AgentMessage::Abort).await;
    info!("abort forwarded");
    (StatusCode::OK, "").into_response()
}

fn find_agent_sender(state: &AppState, vm_id: &str) -> Option<mpsc::Sender<AgentMessage>> {
    state.chat_senders.lock().ok()?.get(vm_id).cloned()
}

async fn validate_csrf(session: &Session, submitted: &str) -> bool {
    session
        .get::<String>("csrf_token")
        .await
        .ok()
        .flatten()
        .is_some_and(|token| token == submitted)
}
