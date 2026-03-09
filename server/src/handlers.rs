use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context};
use axum::{
    extract::{Form, Path, State},
    http::StatusCode,
    response::{Html, IntoResponse, Redirect, Response},
};
use firecracker_manager::create_vm;
use serde::Deserialize;
use tower_sessions::Session;
use tracing::info;
use uuid::Uuid;

use crate::{
    auth::User,
    state::{AppError, AppState, VmEntry, VmInfo},
    templates::{render_terminal_page, render_vms_page},
    vm::{build_vm_config, fetch_host_iam_credentials, find_user_rootfs, user_rootfs_path},
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
    let has_user_rootfs = find_user_rootfs(&state.user_rootfs_dir, &user.email).is_some();
    Ok(Html(render_vms_page(&vms, &csrf_token, has_user_rootfs).into_string()))
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
    let user_rootfs = find_user_rootfs(&state.user_rootfs_dir, &user.email);
    match &user_rootfs {
        Some(path) => info!(email = %user.email, rootfs = %path.display(), "reattaching saved rootfs"),
        None => info!(email = %user.email, rootfs = %state.rootfs_path.display(), "creating rootfs from base image"),
    }
    let vm_config = build_vm_config(&state, iam_creds, user_rootfs.as_deref());
    let vm = create_vm(&vm_config).await?;
    info!(email = %user.email, vm_id = %vm.id, guest_ip = %vm.guest_ip, pid = vm.pid, "vm started");
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
    let entry = {
        let mut registry = state.vms.lock().map_err(|_| anyhow!("vm registry lock poisoned"))?;
        let owned = registry.get(&vm_id).map(|e| e.email == user.email).unwrap_or(false);
        if !owned {
            return Ok((StatusCode::NOT_FOUND, "Not found").into_response());
        }
        registry.remove(&vm_id).expect("just checked")
    };
    tokio::fs::create_dir_all(&state.user_rootfs_dir).await
        .with_context(|| format!("failed to create user rootfs dir {}", state.user_rootfs_dir.display()))?;
    let user_rootfs = user_rootfs_path(&state.user_rootfs_dir, &user.email);
    info!(email = %user.email, vm_id = %vm_id, dest = %user_rootfs.display(), "saving rootfs from deleted vm");
    entry._guard.save_rootfs_to(&user_rootfs).await
        .with_context(|| format!("failed to save rootfs to {}", user_rootfs.display()))?;
    info!(email = %user.email, dest = %user_rootfs.display(), "rootfs saved");
    Ok(Redirect::to("/vms").into_response())
}

pub(crate) async fn delete_user_rootfs_handler(
    user: User,
    session: Session,
    State(state): State<AppState>,
    Form(form): Form<CsrfForm>,
) -> Result<Response, AppError> {
    if !validate_csrf(&session, &form.csrf_token).await {
        return Ok((StatusCode::FORBIDDEN, "Forbidden").into_response());
    }
    let rootfs_path = user_rootfs_path(&state.user_rootfs_dir, &user.email);
    info!(email = %user.email, path = %rootfs_path.display(), "deleting saved rootfs");
    let _ = tokio::fs::remove_file(&rootfs_path).await;
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
