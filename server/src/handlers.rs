use std::time::{SystemTime, UNIX_EPOCH};
use anyhow::anyhow;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{Html, IntoResponse},
    Json,
};
use firecracker_manager::create_vm;

use crate::{
    auth::User,
    frontend::FRONTEND_HTML,
    state::{AppError, AppState, VmEntry, VmInfo},
    vm::{build_vm_config, fetch_host_iam_credentials},
};

pub(crate) async fn get_index(_user: User) -> Html<&'static str> {
    Html(FRONTEND_HTML)
}

pub(crate) async fn list_vms(_user: User, State(state): State<AppState>) -> Result<Json<Vec<VmInfo>>, AppError> {
    let registry = state.vms.lock().map_err(|_| anyhow!("vm registry lock poisoned"))?;
    let mut vms: Vec<VmInfo> = registry
        .iter()
        .map(|(id, e)| VmInfo {
            id: id.clone(),
            guest_ip: e.guest_ip.clone(),
            pid: e.pid,
            created_at: e.created_at,
        })
        .collect();
    vms.sort_by_key(|v| v.created_at);
    Ok(Json(vms))
}

pub(crate) async fn create_vm_handler(
    _user: User,
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    let iam_creds = fetch_host_iam_credentials().await;
    let vm_config = build_vm_config(&state, iam_creds);
    let vm = create_vm(&vm_config).await?;
    let created_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let info = VmInfo {
        id: vm.id.clone(),
        guest_ip: vm.guest_ip.clone(),
        pid: vm.pid,
        created_at,
    };
    state
        .vms
        .lock()
        .map_err(|_| anyhow!("vm registry lock poisoned"))?
        .insert(
            vm.id.clone(),
            VmEntry {
                guest_ip: vm.guest_ip.clone(),
                pid: vm.pid,
                created_at,
                _guard: vm.into_guard(),
            },
        );
    Ok((StatusCode::CREATED, Json(info)))
}

pub(crate) async fn delete_vm_handler(
    _user: User,
    Path(vm_id): Path<String>,
    State(state): State<AppState>,
) -> Result<StatusCode, AppError> {
    let removed = state
        .vms
        .lock()
        .map_err(|_| anyhow!("vm registry lock poisoned"))?
        .remove(&vm_id)
        .is_some();
    if removed {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Ok(StatusCode::NOT_FOUND)
    }
}
