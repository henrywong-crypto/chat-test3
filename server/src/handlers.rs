use std::{sync::Arc, time::Instant};
use anyhow::anyhow;
use axum::{
    extract::{Form, Path, State},
    http::StatusCode,
    response::{Html, IntoResponse, Redirect, Response},
};
use firecracker_manager::create_vm;
use serde::Deserialize;
use store::upsert_user;
use tower_sessions::Session;
use tracing::info;
use uuid::Uuid;

use crate::{
    auth::User,
    state::{AppError, AppState, VmEntry, VmRegistry},
    static_files::{app_js_version, styles_css_version},
    templates::render_terminal_page,
    vm::{
        build_vm_config, ensure_user_rootfs, fetch_host_iam_credentials, find_user_rootfs,
        user_rootfs_path,
    },
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

fn find_user_vm_id(vms: &VmRegistry, user_id: Uuid) -> Option<String> {
    let registry = vms.lock().ok()?;
    registry
        .iter()
        .find(|(_, e)| e.user_id == user_id)
        .map(|(id, _)| id.clone())
}

fn remove_user_vm(vms: &VmRegistry, user_id: Uuid) {
    let removed: Vec<VmEntry> = {
        let Ok(mut registry) = vms.lock() else { return };
        let vm_ids: Vec<String> = registry
            .iter()
            .filter(|(_, e)| e.user_id == user_id)
            .map(|(id, _)| id.clone())
            .collect();
        vm_ids.into_iter().filter_map(|id| registry.remove(&id)).collect()
    };
    drop(removed);
}

pub(crate) async fn get_or_create_terminal(
    user: User,
    session: Session,
    State(state): State<AppState>,
) -> Result<Response, AppError> {
    let db_user = upsert_user(&state.db, &user.email).await?;

    if let Some(vm_id) = find_user_vm_id(&state.vms, db_user.id) {
        return Ok(Redirect::to(&format!("/terminal/{vm_id}")).into_response());
    }

    let vm_count = state.vms.lock().map(|r| r.len()).unwrap_or(0);
    if vm_count >= state.vm_max_count {
        return Ok((StatusCode::SERVICE_UNAVAILABLE, "VM limit reached").into_response());
    }

    let iam_creds = fetch_host_iam_credentials().await;
    let has_iam_creds = iam_creds.is_some();
    let user_rootfs = ensure_user_rootfs(
        &state.user_rootfs_dir,
        &state.rootfs_path,
        db_user.id,
        &state.rootfs_lock,
    )
    .await?;
    info!(user_id = %db_user.id, rootfs = %user_rootfs.display(), "using rootfs");
    let vm_config = build_vm_config(&state, iam_creds, Some(&user_rootfs))?;
    let vm = create_vm(&vm_config).await?;
    info!(user_id = %db_user.id, vm_id = %vm.id, guest_ip = %vm.guest_ip(), pid = vm.pid, "vm started");
    let vm_id = vm.id.clone();
    let vm_entry = VmEntry {
        user_id: db_user.id,
        has_iam_creds,
        created_at: Instant::now(),
        ws_connected: false,
        vm,
    };
    register_vm(&state.vms, vm_id.clone(), vm_entry)?;

    let csrf_token = get_csrf_token(&session).await;
    let has_user_rootfs = find_user_rootfs(&state.user_rootfs_dir, db_user.id).is_some();
    Ok(Html(render_terminal_page(
        &vm_id,
        &csrf_token,
        &state.upload_dir,
        has_user_rootfs,
        app_js_version(),
        styles_css_version(),
    ))
    .into_response())
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
    let db_user = upsert_user(&state.db, &user.email).await?;
    let rootfs_path = user_rootfs_path(&state.user_rootfs_dir, db_user.id);
    info!(user_id = %db_user.id, path = %rootfs_path.display(), "deleting saved rootfs");
    let _guard = state.rootfs_lock.lock().await;
    let _ = tokio::fs::remove_file(&rootfs_path).await;
    drop(_guard);
    remove_user_vm(&state.vms, db_user.id);
    Ok(Redirect::to("/").into_response())
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
    let db_user = match upsert_user(&state.db, &user.email).await {
        Ok(db_user) => db_user,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "Internal error").into_response(),
    };
    let owned = state
        .vms
        .lock()
        .ok()
        .and_then(|r| r.get(&vm_id).map(|e| e.user_id == db_user.id))
        .unwrap_or(false);
    if !owned {
        return (StatusCode::NOT_FOUND, "Not found").into_response();
    }
    let csrf_token = get_csrf_token(&session).await;
    let has_user_rootfs = find_user_rootfs(&state.user_rootfs_dir, db_user.id).is_some();
    Html(render_terminal_page(
        &vm_id,
        &csrf_token,
        &state.upload_dir,
        has_user_rootfs,
        app_js_version(),
        styles_css_version(),
    ))
    .into_response()
}
