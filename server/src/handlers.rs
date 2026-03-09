use anyhow::{anyhow, Context};
use axum::{
    extract::{Form, Path, State},
    http::StatusCode,
    response::{Html, IntoResponse, Redirect, Response},
};
use chrono::Utc;
use firecracker_manager::create_vm;
use serde::Deserialize;
use tower_sessions::Session;
use tracing::info;
use uuid::Uuid;

use crate::{
    auth::User,
    state::{AppError, AppState, VmEntry, VmInfo, VmRegistry},
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

fn validate_vm_id(id: &str) -> bool {
    Uuid::parse_str(id).is_ok()
}

fn register_vm(vms: &VmRegistry, vm_id: String, vm_entry: VmEntry) -> Result<(), AppError> {
    vms.lock()
        .map_err(|_| anyhow!("vm registry lock poisoned"))?
        .insert(vm_id, vm_entry);
    Ok(())
}

fn remove_owned_vm(
    vms: &VmRegistry,
    vm_id: &str,
    email: &str,
) -> Result<Option<VmEntry>, AppError> {
    let mut registry = vms
        .lock()
        .map_err(|_| anyhow!("vm registry lock poisoned"))?;
    let owned = registry
        .get(vm_id)
        .map(|e| e.email == email)
        .unwrap_or(false);
    if !owned {
        return Ok(None);
    }
    Ok(registry.remove(vm_id))
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
    let registry = state
        .vms
        .lock()
        .map_err(|_| anyhow!("vm registry lock poisoned"))?;
    let mut vm_infos: Vec<VmInfo> = registry
        .iter()
        .filter(|(_, e)| e.email == user.email)
        .map(|(id, e)| VmInfo {
            id: id.clone(),
            guest_ip: e.guest_ip.clone(),
            pid: e.pid,
            created_at: e.created_at,
        })
        .collect();
    vm_infos.sort_by_key(|v| v.created_at);
    let has_user_rootfs = find_user_rootfs(&state.user_rootfs_dir, &user.email).is_some();
    Ok(Html(
        render_vms_page(&vm_infos, &csrf_token, has_user_rootfs).into_string(),
    ))
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
    let user_vm_count = state
        .vms
        .lock()
        .map_err(|_| anyhow!("vm registry lock poisoned"))?
        .values()
        .filter(|e| e.email == user.email)
        .count();
    if user_vm_count >= state.max_vms_per_user {
        return Ok((
            StatusCode::FORBIDDEN,
            format!("VM limit reached (max {})", state.max_vms_per_user),
        )
            .into_response());
    }
    let iam_creds = fetch_host_iam_credentials().await;
    let user_rootfs = find_user_rootfs(&state.user_rootfs_dir, &user.email);
    match &user_rootfs {
        Some(path) => {
            info!(email = %user.email, rootfs = %path.display(), "reattaching saved rootfs")
        }
        None => {
            info!(email = %user.email, rootfs = %state.rootfs_path.display(), "creating rootfs from base image")
        }
    }
    let vm_config = build_vm_config(&state, iam_creds, user_rootfs.as_deref());
    let vm = create_vm(&vm_config).await?;
    info!(email = %user.email, vm_id = %vm.id, guest_ip = %vm.guest_ip, pid = vm.pid, "vm started");
    let created_at = Utc::now().timestamp() as u64;
    let vm_id = vm.id.clone();
    let vm_entry = VmEntry {
        guest_ip: vm.guest_ip.clone(),
        pid: vm.pid,
        created_at,
        email: user.email,
        _guard: vm.into_guard(),
    };
    register_vm(&state.vms, vm_id.clone(), vm_entry)?;
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
    if !validate_vm_id(&vm_id) {
        return Ok((StatusCode::NOT_FOUND, "Not found").into_response());
    }
    let Some(vm_entry) = remove_owned_vm(&state.vms, &vm_id, &user.email)? else {
        return Ok((StatusCode::NOT_FOUND, "Not found").into_response());
    };
    tokio::fs::create_dir_all(&state.user_rootfs_dir)
        .await
        .with_context(|| {
            format!(
                "failed to create user rootfs dir {}",
                state.user_rootfs_dir.display()
            )
        })?;
    let user_rootfs = user_rootfs_path(&state.user_rootfs_dir, &user.email);
    info!(email = %user.email, vm_id = %vm_id, dest = %user_rootfs.display(), "saving rootfs from deleted vm");
    vm_entry
        ._guard
        .save_rootfs_to(&user_rootfs)
        .await
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
    session: Session,
    Path(vm_id): Path<String>,
    State(state): State<AppState>,
) -> Response {
    if !validate_vm_id(&vm_id) {
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
    let csrf_token = get_csrf_token(&session).await;
    Html(render_terminal_page(&vm_id, &csrf_token, &state.upload_dir).into_string()).into_response()
}
