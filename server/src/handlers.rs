use anyhow::{anyhow, Result};
use axum::{
    extract::{Form, Multipart, Path, Query, State},
    http::StatusCode,
    response::{Html, IntoResponse, Redirect, Response},
    Json,
};
use firecracker_manager::create_vm;
use serde::Deserialize;
use sftp_client::{open_sftp_session, SftpSession};
use ssh_client::connect_ssh;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use store::upsert_user;
use tokio::io::AsyncWriteExt;
use tower_sessions::Session;
use tracing::{error, info};
use uuid::Uuid;
use vm_lifecycle::{
    build_user_rootfs_path, build_vm_config, ensure_user_rootfs, fetch_host_iam_credentials,
    find_user_rootfs, VmEntry, VmRegistry,
};

use chat_relay::{fetch_transcript, list_sessions};

use crate::{
    auth::User,
    state::{find_vm_guest_ip_for_user, AppError, AppState},
    static_files::{app_js_version, styles_css_version},
    templates::render_terminal_page,
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
        vm_ids
            .into_iter()
            .filter_map(|id| registry.remove(&id))
            .collect()
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
    info!("using rootfs");
    let vm_config = build_vm_config(&state.vm_build_config(), iam_creds, Some(&user_rootfs))?;
    let vm = create_vm(&vm_config).await?;
    info!("vm started");
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
    let rootfs_path = build_user_rootfs_path(&state.user_rootfs_dir, db_user.id);
    info!("deleting saved rootfs");
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

pub(crate) async fn list_chat_sessions_handler(
    user: User,
    Path(vm_id): Path<String>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    if !validate_vm_id(&vm_id) {
        return (StatusCode::NOT_FOUND, "Not found").into_response();
    }
    let db_user = match upsert_user(&state.db, &user.email).await {
        Ok(db_user) => db_user,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "Internal error").into_response(),
    };
    let guest_ip = match find_vm_guest_ip_for_user(&state.vms, &vm_id, db_user.id) {
        Some(ip) => ip,
        None => return (StatusCode::NOT_FOUND, "Session not found or expired").into_response(),
    };
    match list_sessions(
        &guest_ip,
        &state.ssh_key_path,
        &state.ssh_user,
        &state.vm_host_key_path,
    )
    .await
    {
        Ok(session_entries) => Json(session_entries).into_response(),
        Err(e) => {
            error!(vm_id = %vm_id, "list_sessions failed: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "Internal error").into_response()
        }
    }
}

#[derive(Deserialize)]
pub(crate) struct TranscriptQuery {
    session_id: String,
}

pub(crate) async fn get_chat_transcript_handler(
    user: User,
    Path(vm_id): Path<String>,
    Query(query): Query<TranscriptQuery>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    if !validate_vm_id(&vm_id) {
        return (StatusCode::NOT_FOUND, "Not found").into_response();
    }
    let db_user = match upsert_user(&state.db, &user.email).await {
        Ok(db_user) => db_user,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "Internal error").into_response(),
    };
    let guest_ip = match find_vm_guest_ip_for_user(&state.vms, &vm_id, db_user.id) {
        Some(ip) => ip,
        None => return (StatusCode::NOT_FOUND, "Session not found or expired").into_response(),
    };
    match fetch_transcript(
        &guest_ip,
        &state.ssh_key_path,
        &state.ssh_user,
        &state.vm_host_key_path,
        &query.session_id,
    )
    .await
    {
        Ok(transcript) => Json(transcript).into_response(),
        Err(e) => {
            error!(vm_id = %vm_id, session_id = %query.session_id, "fetch_transcript failed: {e}");
            (StatusCode::NOT_FOUND, "Transcript not found").into_response()
        }
    }
}

pub(crate) async fn handle_chat_upload(
    user: User,
    session: Session,
    Path(vm_id): Path<String>,
    State(state): State<AppState>,
    multipart: Multipart,
) -> Result<Response, AppError> {
    if !validate_vm_id(&vm_id) {
        return Ok((StatusCode::NOT_FOUND, "Not found").into_response());
    }
    let db_user = upsert_user(&state.db, &user.email).await?;
    let (csrf_token, filename, file_bytes) = extract_chat_upload_fields(multipart).await?;
    if !validate_csrf(&session, &csrf_token).await {
        return Ok((StatusCode::FORBIDDEN, "Forbidden").into_response());
    }
    let guest_ip = match find_vm_guest_ip_for_user(&state.vms, &vm_id, db_user.id) {
        Some(ip) => ip,
        None => return Ok((StatusCode::NOT_FOUND, "Session not found or expired").into_response()),
    };
    let remote_path = build_chat_upload_path(&filename);
    info!("uploading chat attachment via sftp");
    let mut ssh_handle = connect_ssh(
        &guest_ip,
        &state.ssh_key_path,
        &state.ssh_user,
        &state.vm_host_key_path,
    )
    .await?;
    let sftp = open_sftp_session(&mut ssh_handle).await?;
    write_chat_file_via_sftp(sftp, &remote_path, &file_bytes).await?;
    Ok(Json(serde_json::json!({"path": remote_path})).into_response())
}

async fn extract_chat_upload_fields(mut multipart: Multipart) -> Result<(String, String, Vec<u8>)> {
    let mut csrf_token: Option<String> = None;
    let mut filename: Option<String> = None;
    let mut file_bytes: Option<Vec<u8>> = None;
    while let Some(field) = multipart.next_field().await.map_err(|e| anyhow!("{e}"))? {
        match field.name() {
            Some("csrf_token") => {
                csrf_token = Some(field.text().await.map_err(|e| anyhow!("{e}"))?);
            }
            Some("file") => {
                let orig_name = field.file_name().unwrap_or("upload").to_owned();
                filename = Some(orig_name);
                file_bytes = Some(field.bytes().await.map_err(|e| anyhow!("{e}"))?.to_vec());
            }
            _ => {}
        }
    }
    let csrf_token = csrf_token.ok_or_else(|| anyhow!("missing csrf_token field"))?;
    let filename = filename.ok_or_else(|| anyhow!("missing file field"))?;
    let file_bytes = file_bytes.ok_or_else(|| anyhow!("missing file bytes"))?;
    Ok((csrf_token, filename, file_bytes))
}

fn build_chat_upload_path(filename: &str) -> String {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let safe_name: String = filename
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '.' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("/tmp/{ts}_{safe_name}")
}

async fn write_chat_file_via_sftp(sftp: SftpSession, path: &str, data: &[u8]) -> Result<()> {
    let mut file = sftp
        .create(path)
        .await
        .map_err(|e| anyhow!("sftp create: {e}"))?;
    file.write_all(data)
        .await
        .map_err(|e| anyhow!("sftp write: {e}"))?;
    file.shutdown()
        .await
        .map_err(|e| anyhow!("sftp shutdown: {e}"))?;
    Ok(())
}
