use anyhow::anyhow;
use axum::{
    Json,
    body::Body,
    extract::State,
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use chat_relay::{AgentMessage, send_agent_message, stream_agent_sse};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use std::convert::Infallible;
use tower_sessions::Session;
use tracing::{error, info};
use uuid::Uuid;

use crate::{
    handlers::{UserVm, UserVmById, attach_csrf_token, validate_csrf},
    state::{AppError, AppState, update_vm_last_activity},
};

pub(crate) async fn handle_chat_stream(
    user_vm: UserVm,
    State(state): State<AppState>,
) -> Result<Response, AppError> {
    update_vm_last_activity(&state.vms, &user_vm.vm_id)?;
    let event_stream = stream_agent_sse(
        user_vm.guest_ip,
        state.config.ssh_key_path.clone(),
        state.config.ssh_user.clone(),
        state.config.vm_host_key_path.clone(),
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

async fn dispatch_agent_message(
    user_vm: &UserVmById,
    state: &AppState,
    message: &AgentMessage,
) -> Result<(), Response> {
    send_agent_message(
        user_vm.guest_ip,
        &state.config.ssh_key_path,
        &state.config.ssh_user,
        &state.config.vm_host_key_path,
        message,
    )
    .await
    .map_err(|e| {
        error!("failed to send agent message: {e}");
        (StatusCode::SERVICE_UNAVAILABLE, "Agent not available").into_response()
    })
}

#[derive(Deserialize)]
pub(crate) struct QueryBody {
    content: String,
    session_id: Option<String>,
    work_dir: Option<String>,
    csrf_token: String,
}

#[derive(Serialize)]
struct QueryResponse {
    task_id: String,
}

pub(crate) async fn handle_chat_query(
    user_vm: UserVmById,
    session: Session,
    State(state): State<AppState>,
    Json(body): Json<QueryBody>,
) -> Response {
    let Some(csrf_token) = validate_csrf(&session, &body.csrf_token).await else {
        return (StatusCode::FORBIDDEN, "Forbidden").into_response();
    };
    let task_id = Uuid::new_v4().to_string();
    let content_len = body.content.len();
    let agent_message = AgentMessage::Query {
        task_id: task_id.clone(),
        content: body.content,
        session_id: body.session_id,
        work_dir: body.work_dir,
    };
    if let Err(response) = dispatch_agent_message(&user_vm, &state, &agent_message).await {
        return response;
    }
    info!("query forwarded  task_id={task_id}  content_len={content_len}");
    attach_csrf_token(
        (StatusCode::OK, Json(QueryResponse { task_id })).into_response(),
        &csrf_token,
    )
}

#[derive(Deserialize)]
pub(crate) struct QuestionAnswerBody {
    request_id: String,
    answers: serde_json::Value,
    csrf_token: String,
}

pub(crate) async fn handle_chat_question_answer(
    user_vm: UserVmById,
    session: Session,
    State(state): State<AppState>,
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
    if let Err(response) = dispatch_agent_message(&user_vm, &state, &agent_message).await {
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
    user_vm: UserVmById,
    session: Session,
    State(state): State<AppState>,
    Json(body): Json<StopBody>,
) -> Response {
    let Some(csrf_token) = validate_csrf(&session, &body.csrf_token).await else {
        return (StatusCode::FORBIDDEN, "Forbidden").into_response();
    };
    let agent_message = AgentMessage::Interrupt { task_id: body.task_id };
    if let Err(response) = dispatch_agent_message(&user_vm, &state, &agent_message).await {
        return response;
    }
    info!("interrupt forwarded");
    attach_csrf_token((StatusCode::OK, "").into_response(), &csrf_token)
}

#[derive(Deserialize)]
pub(crate) struct HelloBody {
    task_id: String,
    csrf_token: String,
}

pub(crate) async fn handle_chat_hello(
    user_vm: UserVmById,
    session: Session,
    State(state): State<AppState>,
    Json(body): Json<HelloBody>,
) -> Response {
    let Some(csrf_token) = validate_csrf(&session, &body.csrf_token).await else {
        return (StatusCode::FORBIDDEN, "Forbidden").into_response();
    };
    let agent_message = AgentMessage::Hello { task_id: body.task_id };
    if let Err(response) = dispatch_agent_message(&user_vm, &state, &agent_message).await {
        return response;
    }
    attach_csrf_token((StatusCode::OK, "").into_response(), &csrf_token)
}
