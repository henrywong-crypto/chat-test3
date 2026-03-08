use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::anyhow;
use axum::{
    extract::{Path, State},
    response::{Html, IntoResponse, Redirect},
};
use firecracker_manager::create_vm;

use crate::{
    auth::User,
    state::{AppError, AppState, VmEntry, VmInfo},
    templates::{render_terminal_page, render_vms_page},
    vm::{build_vm_config, fetch_host_iam_credentials},
};

pub(crate) async fn get_redirect_to_vms(_user: User) -> Redirect {
    Redirect::to("/vms")
}

pub(crate) async fn get_vms_page(
    _user: User,
    State(state): State<AppState>,
) -> Result<Html<String>, AppError> {
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
    Ok(Html(render_vms_page(&vms).into_string()))
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
                _guard: vm.into_guard(),
            },
        );
    Ok(Redirect::to(&format!("/terminal/{vm_id}")))
}

pub(crate) async fn delete_vm_handler(
    _user: User,
    Path(vm_id): Path<String>,
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    state
        .vms
        .lock()
        .map_err(|_| anyhow!("vm registry lock poisoned"))?
        .remove(&vm_id);
    Ok(Redirect::to("/vms"))
}

pub(crate) async fn get_terminal_page(
    _user: User,
    Path(vm_id): Path<String>,
) -> Html<String> {
    Html(render_terminal_page(&vm_id).into_string())
}
