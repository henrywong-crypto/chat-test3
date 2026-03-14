use anyhow::{Context, Result, anyhow};
use axum::{
    Json,
    extract::{Form, Multipart, Path as RoutePath, Query, State},
    http::StatusCode,
    response::{Html, IntoResponse, Redirect, Response},
};
use chat_history::{delete_chat_session, fetch_chat_history, list_chat_sessions};
use firecracker_manager::create_vm;
use futures::TryStreamExt;
use russh_sftp::client::SftpSession;
use serde::Deserialize;
use sftp_client::open_sftp_session;
use ssh_client::connect_ssh;
use std::{
    io::{Error as IoError, ErrorKind},
    path::Path,
    time::{Instant, SystemTime, UNIX_EPOCH},
};
use store::{User as DbUser, upsert_user};
use tokio::io::{AsyncRead, AsyncWriteExt};
use tokio_util::io::StreamReader;
use tower_sessions::Session;
use tracing::{error, info};
use uuid::Uuid;
use vm_lifecycle::{
    VmEntry, VmRegistry, build_user_rootfs_path, build_vm_config, ensure_user_rootfs,
    fetch_host_iam_credentials, find_user_rootfs,
};

use crate::{
    auth::User,
    state::{AppError, AppState, find_vm_guest_ip_for_user},
    static_files::{app_js_version, styles_css_version},
    templates::render_terminal_page,
};

#[derive(Deserialize)]
pub(crate) struct CsrfForm {
    csrf_token: String,
}

async fn get_csrf_token(session: &Session) -> Result<String> {
    let token = session
        .get::<String>("csrf_token")
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| Uuid::new_v4().to_string().replace('-', ""));
    session
        .insert("csrf_token", &token)
        .await
        .context("failed to store CSRF token")?;
    Ok(token)
}

async fn validate_csrf(session: &Session, submitted: &str) -> bool {
    session
        .get::<String>("csrf_token")
        .await
        .ok()
        .flatten()
        .is_some_and(|token| token == submitted)
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

fn find_user_vm_id(vms: &VmRegistry, user_id: Uuid) -> Result<Option<String>> {
    let registry = vms
        .lock()
        .map_err(|_| anyhow!("vm registry lock poisoned"))?;
    Ok(registry
        .iter()
        .find(|(_, e)| e.user_id == user_id)
        .map(|(id, _)| id.clone()))
}

fn remove_user_vm(vms: &VmRegistry, user_id: Uuid) -> Result<()> {
    let mut registry = vms
        .lock()
        .map_err(|_| anyhow!("vm registry lock poisoned"))?;
    let vm_ids: Vec<String> = registry
        .iter()
        .filter(|(_, e)| e.user_id == user_id)
        .map(|(id, _)| id.clone())
        .collect();
    let removed: Vec<VmEntry> = vm_ids
        .into_iter()
        .filter_map(|id| registry.remove(&id))
        .collect();
    drop(removed);
    Ok(())
}

pub(crate) async fn get_or_create_terminal(
    user: User,
    session: Session,
    State(state): State<AppState>,
) -> Result<Response, AppError> {
    let db_user = upsert_user(&state.db, &user.email).await?;

    let Some(vm_id) = find_user_vm_id(&state.vms, db_user.id)? else {
        return check_vm_limit_and_create(session, state, db_user).await;
    };
    Ok(Redirect::to(&format!("/terminal/{vm_id}")).into_response())
}

async fn check_vm_limit_and_create(
    session: Session,
    state: AppState,
    db_user: DbUser,
) -> Result<Response, AppError> {
    let vm_count = state
        .vms
        .lock()
        .map_err(|_| anyhow!("vm registry lock poisoned"))?
        .len();
    if vm_count >= state.vm_max_count {
        return Ok((StatusCode::SERVICE_UNAVAILABLE, "VM limit reached").into_response());
    }

    let iam_creds = fetch_host_iam_credentials(&state.iam_role_name)
        .await
        .context("failed to fetch IAM credentials for VM")?;
    info!("building vm config");
    let user_rootfs = ensure_user_rootfs(
        &state.user_rootfs_dir,
        &state.rootfs_path,
        db_user.id,
        &state.rootfs_lock,
    )
    .await?;
    info!("using rootfs");
    let vm_config = build_vm_config(&state.vm_build_config(), iam_creds, &user_rootfs)?;
    let vm = create_vm(&vm_config).await?;
    info!("vm started");
    let vm_id = vm.id.clone();
    let vm_entry = VmEntry {
        user_id: db_user.id,
        has_iam_creds: true,
        created_at: Instant::now(),
        ws_connected: false,
        vm,
    };
    register_vm(&state.vms, vm_id.clone(), vm_entry)?;

    let csrf_token = get_csrf_token(&session).await?;
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
    remove_user_vm(&state.vms, db_user.id)?;
    Ok(Redirect::to("/").into_response())
}

pub(crate) async fn get_terminal_page(
    user: User,
    session: Session,
    RoutePath(vm_id): RoutePath<String>,
    State(state): State<AppState>,
) -> Result<Response, AppError> {
    if !validate_vm_id(&vm_id) {
        return Ok((StatusCode::NOT_FOUND, "Not found").into_response());
    }
    let db_user = upsert_user(&state.db, &user.email).await?;
    let owned = state
        .vms
        .lock()
        .map_err(|_| anyhow!("vm registry lock poisoned"))?
        .get(&vm_id)
        .map(|e| e.user_id == db_user.id)
        .unwrap_or(false);
    if !owned {
        return Ok((StatusCode::NOT_FOUND, "Not found").into_response());
    }
    let csrf_token = get_csrf_token(&session).await?;
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

pub(crate) async fn list_chat_sessions_handler(
    user: User,
    RoutePath(vm_id): RoutePath<String>,
    State(state): State<AppState>,
) -> Result<Response, AppError> {
    if !validate_vm_id(&vm_id) {
        return Ok((StatusCode::NOT_FOUND, "Not found").into_response());
    }
    let db_user = upsert_user(&state.db, &user.email).await?;
    let Some(guest_ip) = find_vm_guest_ip_for_user(&state.vms, &vm_id, db_user.id)? else {
        return Ok((StatusCode::NOT_FOUND, "Session not found or expired").into_response());
    };
    Ok(list_chat_sessions(
        &guest_ip,
        &state.ssh_key_path,
        &state.ssh_user,
        &state.vm_host_key_path,
        Path::new(&state.ssh_user_home),
    )
    .await
    .map(|sessions| Json(sessions).into_response())
    .unwrap_or_else(|e| {
        error!("list_chat_sessions failed: {e}");
        (StatusCode::INTERNAL_SERVER_ERROR, "Internal error").into_response()
    }))
}

#[derive(Deserialize)]
pub(crate) struct TranscriptQuery {
    session_id: String,
    project_dir: String,
}

pub(crate) async fn get_chat_transcript_handler(
    user: User,
    RoutePath(vm_id): RoutePath<String>,
    Query(query): Query<TranscriptQuery>,
    State(state): State<AppState>,
) -> Result<Response, AppError> {
    if !validate_vm_id(&vm_id) {
        return Ok((StatusCode::NOT_FOUND, "Not found").into_response());
    }
    let db_user = upsert_user(&state.db, &user.email).await?;
    let Some(guest_ip) = find_vm_guest_ip_for_user(&state.vms, &vm_id, db_user.id)? else {
        return Ok((StatusCode::NOT_FOUND, "Session not found or expired").into_response());
    };
    Ok(fetch_chat_history(
        &guest_ip,
        &state.ssh_key_path,
        &state.ssh_user,
        &state.vm_host_key_path,
        &query.session_id,
        Path::new(&query.project_dir),
    )
    .await
    .map(|history| Json(history).into_response())
    .unwrap_or_else(|e| {
        error!("fetch_chat_history failed: {e}");
        (StatusCode::NOT_FOUND, "Transcript not found").into_response()
    }))
}

#[derive(Deserialize)]
pub(crate) struct DeleteChatSessionForm {
    csrf_token: String,
    session_id: String,
    project_dir: String,
}

pub(crate) async fn delete_chat_session_handler(
    user: User,
    RoutePath(vm_id): RoutePath<String>,
    session: Session,
    State(state): State<AppState>,
    Form(form): Form<DeleteChatSessionForm>,
) -> Result<Response, AppError> {
    if !validate_vm_id(&vm_id) {
        return Ok((StatusCode::NOT_FOUND, "Not found").into_response());
    }
    if !validate_csrf(&session, &form.csrf_token).await {
        return Ok((StatusCode::FORBIDDEN, "Forbidden").into_response());
    }
    let db_user = upsert_user(&state.db, &user.email).await?;
    let Some(guest_ip) = find_vm_guest_ip_for_user(&state.vms, &vm_id, db_user.id)? else {
        return Ok((StatusCode::NOT_FOUND, "Session not found or expired").into_response());
    };
    delete_chat_session(
        &guest_ip,
        &state.ssh_key_path,
        &state.ssh_user,
        &state.vm_host_key_path,
        &form.session_id,
        Path::new(&form.project_dir),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

struct ChatUploadMetadata {
    csrf_token: String,
}

pub(crate) async fn handle_chat_upload(
    user: User,
    session: Session,
    RoutePath(vm_id): RoutePath<String>,
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<Response, AppError> {
    if !validate_vm_id(&vm_id) {
        return Ok((StatusCode::NOT_FOUND, "Not found").into_response());
    }
    let db_user = upsert_user(&state.db, &user.email).await?;
    let chat_upload_metadata = extract_chat_upload_metadata(&mut multipart).await?;
    if !validate_csrf(&session, &chat_upload_metadata.csrf_token).await {
        return Ok((StatusCode::FORBIDDEN, "Forbidden").into_response());
    }
    let Some(guest_ip) = find_vm_guest_ip_for_user(&state.vms, &vm_id, db_user.id)? else {
        return Ok((StatusCode::NOT_FOUND, "Session not found or expired").into_response());
    };
    info!("uploading chat attachment via sftp");
    let mut ssh_handle = connect_ssh(
        &guest_ip,
        &state.ssh_key_path,
        &state.ssh_user,
        &state.vm_host_key_path,
    )
    .await?;
    let sftp = open_sftp_session(&mut ssh_handle).await?;
    let remote_path = stream_chat_attachment(&mut multipart, sftp).await?;
    Ok(Json(serde_json::json!({"path": remote_path})).into_response())
}

async fn extract_chat_upload_metadata(multipart: &mut Multipart) -> Result<ChatUploadMetadata> {
    while let Some(field) = multipart
        .next_field()
        .await
        .context("failed to read multipart field")?
    {
        let name = field.name().unwrap_or("").to_owned();
        if name == "csrf_token" {
            let csrf_token = field
                .text()
                .await
                .context("failed to read csrf_token field")?;
            return Ok(ChatUploadMetadata { csrf_token });
        }
    }
    Err(anyhow!("missing 'csrf_token' field"))
}

async fn stream_chat_attachment(multipart: &mut Multipart, sftp: SftpSession) -> Result<String> {
    while let Some(field) = multipart
        .next_field()
        .await
        .context("failed to read multipart field")?
    {
        if field.name().unwrap_or("") == "file" {
            let filename = field
                .file_name()
                .context("file upload missing filename")?
                .to_owned();
            let remote_path = build_chat_upload_path(&filename);
            let mut reader =
                StreamReader::new(field.map_err(|e| IoError::new(ErrorKind::Other, e)));
            write_chat_file_via_sftp(sftp, &remote_path, &mut reader).await?;
            return Ok(remote_path);
        }
    }
    Err(anyhow!("missing 'file' field"))
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

async fn write_chat_file_via_sftp(
    sftp: SftpSession,
    path: &str,
    reader: &mut (impl AsyncRead + Unpin),
) -> Result<()> {
    let mut file = sftp
        .create(path)
        .await
        .map_err(|e| anyhow!("sftp create: {e}"))?;
    tokio::io::copy(reader, &mut file)
        .await
        .context("failed to write chat file via sftp")?;
    file.shutdown()
        .await
        .map_err(|e| anyhow!("sftp shutdown: {e}"))?;
    Ok(())
}
