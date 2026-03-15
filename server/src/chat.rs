use anyhow::anyhow;
use axum::{
    Json,
    body::Body,
    extract::{FromRequestParts, Path, State},
    http::{StatusCode, header, request::Parts},
    response::{IntoResponse, Response},
};
use chat_relay::{AgentMessage, start_agent_relay};
use futures::StreamExt;
use serde::Deserialize;
use std::{convert::Infallible, time::Duration};
use tokio::{sync::mpsc, time::timeout};
use tower_sessions::Session;
use tracing::{error, info};
use uuid::Uuid;

use crate::{
    handlers::{UserVm, attach_csrf_token, validate_csrf},
    state::{AppError, AppState, update_vm_last_activity},
};

const SEND_TIMEOUT_SECS: u64 = 30;

pub(crate) async fn handle_chat_stream(
    user_vm: UserVm,
    Path(vm_id): Path<String>,
    State(state): State<AppState>,
) -> Result<Response, AppError> {
    if Uuid::parse_str(&vm_id).is_err() {
        return Ok((StatusCode::NOT_FOUND, "Not found").into_response());
    }
    update_vm_last_activity(&state.vms, &user_vm.vm_id)?;
    let (agent_tx, agent_rx) = mpsc::channel::<AgentMessage>(4);
    state
        .chat_senders
        .lock()
        .map_err(|e| anyhow!("chat senders lock poisoned: {e}"))?
        .insert(vm_id.clone(), agent_tx);
    let event_stream = start_agent_relay(
        user_vm.guest_ip,
        &state.config.ssh_key_path,
        &state.config.ssh_user,
        &state.config.vm_host_key_path,
        agent_rx,
    );
    info!("chat sse stream opened");
    let body = Body::from_stream(event_stream.map(Ok::<_, Infallible>));
    Response::builder()
        .header(header::CONTENT_TYPE, "text/event-stream")
        .header(header::CACHE_CONTROL, "no-cache")
        .header("x-accel-buffering", "no")
        .body(body)
        .map_err(|e| anyhow!("failed to build SSE response: {e}"))
        .map_err(AppError::from)
}

fn find_agent_sender(state: &AppState, vm_id: &str) -> Option<mpsc::Sender<AgentMessage>> {
    let mut senders = state.chat_senders.lock().ok()?;
    let sender = senders.get(vm_id)?;
    if sender.is_closed() {
        senders.remove(vm_id);
        return None;
    }
    Some(sender.clone())
}

pub(crate) struct VerifiedVmSender(mpsc::Sender<AgentMessage>);

impl FromRequestParts<AppState> for VerifiedVmSender {
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        UserVm::from_request_parts(parts, state).await?;
        let Path(vm_id) = Path::<String>::from_request_parts(parts, state).await
            .map_err(IntoResponse::into_response)?;
        if Uuid::parse_str(&vm_id).is_err() {
            return Err((StatusCode::NOT_FOUND, "Not found").into_response());
        }
        let Some(agent_tx) = find_agent_sender(state, &vm_id) else {
            info!("no active chat stream");
            return Err((StatusCode::NOT_FOUND, "No active chat stream").into_response());
        };
        Ok(VerifiedVmSender(agent_tx))
    }
}

async fn forward_agent_message(
    agent_tx: mpsc::Sender<AgentMessage>,
    agent_message: AgentMessage,
) -> Result<(), Response> {
    match timeout(Duration::from_secs(SEND_TIMEOUT_SECS), agent_tx.send(agent_message)).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(_)) => {
            info!("agent sender closed");
            Err((StatusCode::SERVICE_UNAVAILABLE, "Agent not available").into_response())
        }
        Err(_) => {
            error!("send timed out, agent likely stuck");
            Err((StatusCode::SERVICE_UNAVAILABLE, "Agent not available").into_response())
        }
    }
}

#[derive(Deserialize)]
pub(crate) struct QueryBody {
    content: String,
    session_id: Option<String>,
    csrf_token: String,
}

pub(crate) async fn handle_chat_query(
    VerifiedVmSender(agent_tx): VerifiedVmSender,
    session: Session,
    Json(body): Json<QueryBody>,
) -> Response {
    let Some(csrf_token) = validate_csrf(&session, &body.csrf_token).await else {
        return (StatusCode::FORBIDDEN, "Forbidden").into_response();
    };
    let content_len = body.content.len();
    let agent_message = AgentMessage::Query {
        content: body.content,
        session_id: body.session_id,
    };
    if let Err(response) = forward_agent_message(agent_tx, agent_message).await {
        return response;
    }
    info!("query forwarded  content_len={content_len}");
    attach_csrf_token((StatusCode::OK, "").into_response(), &csrf_token)
}

#[derive(Deserialize)]
pub(crate) struct QuestionAnswerBody {
    request_id: String,
    answers: serde_json::Value,
    csrf_token: String,
}

pub(crate) async fn handle_chat_question_answer(
    VerifiedVmSender(agent_tx): VerifiedVmSender,
    session: Session,
    Json(body): Json<QuestionAnswerBody>,
) -> Response {
    let Some(csrf_token) = validate_csrf(&session, &body.csrf_token).await else {
        return (StatusCode::FORBIDDEN, "Forbidden").into_response();
    };
    let request_id = body.request_id.clone();
    let agent_message = AgentMessage::QuestionAnswer {
        request_id: body.request_id,
        answers: body.answers,
    };
    if let Err(response) = forward_agent_message(agent_tx, agent_message).await {
        return response;
    }
    info!("question answer forwarded  request_id={request_id}");
    attach_csrf_token((StatusCode::OK, "").into_response(), &csrf_token)
}

#[derive(Deserialize)]
pub(crate) struct StopBody {
    task_id: String,
    csrf_token: String,
}

pub(crate) async fn handle_chat_stop(
    VerifiedVmSender(agent_tx): VerifiedVmSender,
    session: Session,
    Json(body): Json<StopBody>,
) -> Response {
    let Some(csrf_token) = validate_csrf(&session, &body.csrf_token).await else {
        return (StatusCode::FORBIDDEN, "Forbidden").into_response();
    };
    if let Err(response) = forward_agent_message(agent_tx, AgentMessage::Interrupt { task_id: body.task_id }).await {
        return response;
    }
    info!("interrupt forwarded");
    attach_csrf_token((StatusCode::OK, "").into_response(), &csrf_token)
}

