use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::anyhow;
use axum::{
    extract::{Form, Path, State},
    http::StatusCode,
    response::{Html, IntoResponse, Redirect, Response},
};
use firecracker_manager::create_vm;
use serde::Deserialize;
use tower_sessions::Session;
use uuid::Uuid;

use crate::{
    auth::User,
    state::{AppError, AppState, VmEntry, VmInfo},
    templates::{render_terminal_page, render_vms_page},
    vm::{build_vm_config, fetch_host_iam_credentials},
};

#[derive(Deserialize)]
pub(crate) struct CsrfForm {
    csrf_token: String,
}

async fn get_csrf_token(session: &Session) -> String {
    if let Ok(Some(token)) = session.get::<String>("csrf_token").await {
        return token;
    }
    let token = Uuid::new_v4().to_string().replace('-', "");
    let _ = session.insert("csrf_token", &token).await;
    token
}

async fn validate_csrf(session: &Session, submitted: &str) -> bool {
    match session.get::<String>("csrf_token").await {
        Ok(Some(token)) => token == submitted,
        _ => false,
    }
}

fn is_valid_vm_id(id: &str) -> bool {
    Uuid::parse_str(id).is_ok()
}

pub(crate) async fn get_redirect_to_vms(_user: User) -> Redirect {
    Redirect::to("/vms")
}

pub(crate) async fn get_vms_page(
    user: User,
    session: Session,
    State(state): State<AppState>,
) -> Result<Html<String>, AppError> {
    let csrf_token = get_csrf_token(&session).await;
    let registry = state.vms.lock().map_err(|_| anyhow!("vm registry lock poisoned"))?;
    let mut vms: Vec<VmInfo> = registry
        .iter()
        .filter(|(_, e)| e.email == user.email)
        .map(|(id, e)| VmInfo {
            id: id.clone(),
            guest_ip: e.guest_ip.clone(),
            pid: e.pid,
            created_at: e.created_at,
        })
        .collect();
    vms.sort_by_key(|v| v.created_at);
    Ok(Html(render_vms_page(&vms, &csrf_token).into_string()))
}

pub(crate) async fn create_vm_handler(
    user: User,
    session: Session,
    State(state): State<AppState>,
    Form(form): Form<CsrfForm>,
) -> Result<Response, AppError> {
    if !validate_csrf(&session, &form.csrf_token).await {
        return Ok((StatusCode::FORBIDDEN, "Forbidden").into_response());
    }
    let iam_creds = fetch_host_iam_credentials().await;
    let vm_config = build_vm_config(&state, iam_creds);
    let vm = create_vm(&vm_config).await?;
    let created_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let vm_id = vm.id.clone();
    state
        .vms
        .lock()
        .map_err(|_| anyhow!("vm registry lock poisoned"))?
        .insert(
            vm_id.clone(),
            VmEntry {
                guest_ip: vm.guest_ip.clone(),
                pid: vm.pid,
                created_at,
                email: user.email,
                _guard: vm.into_guard(),
            },
        );
    Ok(Redirect::to(&format!("/terminal/{vm_id}")).into_response())
}

pub(crate) async fn delete_vm_handler(
    user: User,
    session: Session,
    Path(vm_id): Path<String>,
    State(state): State<AppState>,
    Form(form): Form<CsrfForm>,
) -> Result<Response, AppError> {
    if !validate_csrf(&session, &form.csrf_token).await {
        return Ok((StatusCode::FORBIDDEN, "Forbidden").into_response());
    }
    if !is_valid_vm_id(&vm_id) {
        return Ok((StatusCode::NOT_FOUND, "Not found").into_response());
    }
    let mut registry = state.vms.lock().map_err(|_| anyhow!("vm registry lock poisoned"))?;
    let owned = registry.get(&vm_id).map(|e| e.email == user.email).unwrap_or(false);
    if !owned {
        return Ok((StatusCode::NOT_FOUND, "Not found").into_response());
    }
    registry.remove(&vm_id);
    Ok(Redirect::to("/vms").into_response())
}

pub(crate) async fn get_terminal_page(
    user: User,
    Path(vm_id): Path<String>,
    State(state): State<AppState>,
) -> Response {
    if !is_valid_vm_id(&vm_id) {
        return (StatusCode::NOT_FOUND, "Not found").into_response();
    }
    let owned = state
        .vms
        .lock()
        .ok()
        .and_then(|r| r.get(&vm_id).map(|e| e.email == user.email))
        .unwrap_or(false);
    if !owned {
        return (StatusCode::NOT_FOUND, "Not found").into_response();
    }
    Html(render_terminal_page(&vm_id).into_string()).into_response()
}
