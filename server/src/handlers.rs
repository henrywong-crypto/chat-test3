use anyhow::{Context, Result, anyhow};
use axum::{
    Json,
    extract::{Form, FromRequestParts, Multipart, Path as RoutePath, Query, State},
    http::{StatusCode, request::Parts},
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
    net::Ipv4Addr,
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use store::upsert_user;
use tokio::{io::{AsyncRead, AsyncWriteExt}, time::timeout};
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
    state::{AppError, AppState, find_user_vm, find_vm_guest_ip_for_user},
    static_files::{app_js_version, styles_css_version},
    templates::render_terminal_page,
};

const LOCK_TIMEOUT_SECS: u64 = 30;

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

pub(crate) struct UserVm {
    pub(crate) user_id: Uuid,
    pub(crate) vm_id: String,
    pub(crate) guest_ip: Ipv4Addr,
}

impl FromRequestParts<AppState> for UserVm {
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let user = User::from_request_parts(parts, state).await
            .map_err(IntoResponse::into_response)?;
        let db_user = upsert_user(&state.db, &user.email).await.map_err(|e| {
            error!("db error: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "An internal error occurred").into_response()
        })?;
        let (vm_id, guest_ip) = match find_user_vm(&state.vms, db_user.id).map_err(|e| {
            error!("vm registry error: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "An internal error occurred").into_response()
        })? {
            Some(entry) => entry,
            None => {
                let vm_count = count_registered_vms(&state.vms)
                    .map_err(IntoResponse::into_response)?;
                if vm_count >= state.config.vm_max_count {
                    return Err((StatusCode::SERVICE_UNAVAILABLE, "VM limit reached").into_response());
                }
                let new_vm_id = provision_new_vm(state, db_user.id).await
                    .map_err(IntoResponse::into_response)?;
                let guest_ip = find_vm_guest_ip_for_user(&state.vms, &new_vm_id, db_user.id)
                    .map_err(|e| {
                        error!("vm registry error after provision: {e}");
                        (StatusCode::INTERNAL_SERVER_ERROR, "An internal error occurred").into_response()
                    })?
                    .ok_or_else(|| {
                        error!("newly provisioned VM not found in registry");
                        (StatusCode::INTERNAL_SERVER_ERROR, "An internal error occurred").into_response()
                    })?;
                (new_vm_id, guest_ip)
            }
        };
        Ok(UserVm { user_id: db_user.id, vm_id, guest_ip })
    }
}

pub(crate) struct UserVmById {
    pub(crate) vm_id: String,
    pub(crate) user_id: Uuid,
    pub(crate) guest_ip: Ipv4Addr,
}

impl FromRequestParts<AppState> for UserVmById {
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let RoutePath(vm_id) = RoutePath::<String>::from_request_parts(parts, state).await
            .map_err(IntoResponse::into_response)?;
        if !validate_vm_id(&vm_id) {
            return Err((StatusCode::NOT_FOUND, "Not found").into_response());
        }
        let user = User::from_request_parts(parts, state).await
            .map_err(IntoResponse::into_response)?;
        let db_user = upsert_user(&state.db, &user.email).await.map_err(|e| {
            error!("db error: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "An internal error occurred").into_response()
        })?;
        let Some(guest_ip) = find_vm_guest_ip_for_user(&state.vms, &vm_id, db_user.id).map_err(|e| {
            error!("vm registry error: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "An internal error occurred").into_response()
        })? else {
            return Err((StatusCode::NOT_FOUND, "Session not found or expired").into_response());
        };
        Ok(UserVmById { vm_id, user_id: db_user.id, guest_ip })
    }
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
    user_vm: UserVm,
    session: Session,
    State(state): State<AppState>,
) -> Result<Response, AppError> {
    build_terminal_response(&session, &state, user_vm.user_id, &user_vm.vm_id).await
}

pub(crate) fn count_registered_vms(vms: &VmRegistry) -> Result<usize, AppError> {
    Ok(vms
        .lock()
        .map_err(|_| anyhow!("vm registry lock poisoned"))?
        .len())
}

pub(crate) async fn provision_new_vm(state: &AppState, user_id: Uuid) -> Result<String, AppError> {
    let iam_creds = fetch_host_iam_credentials(&state.config.iam_role_name)
        .await
        .context("failed to fetch IAM credentials for VM")?;
    info!("building vm config");
    let user_rootfs = ensure_user_rootfs(
        &state.config.user_rootfs_dir,
        &state.config.rootfs_path,
        user_id,
        &state.rootfs_lock,
    )
    .await?;
    info!("using rootfs");
    let vm_config = build_vm_config(&state.config.vm_build_config(), &iam_creds, &user_rootfs)?;
    let vm = create_vm(&vm_config).await?;
    info!("vm started");
    let vm_id = vm.id.clone();
    register_vm(
        &state.vms,
        vm_id.clone(),
        VmEntry {
            user_id,
            has_iam_creds: true,
            last_activity: Instant::now(),
            vm,
        },
    )?;
    Ok(vm_id)
}

async fn build_terminal_response(
    session: &Session,
    state: &AppState,
    user_id: Uuid,
    vm_id: &str,
) -> Result<Response, AppError> {
    let csrf_token = get_csrf_token(session).await?;
    let has_user_rootfs = find_user_rootfs(&state.config.user_rootfs_dir, user_id).is_some();
    Ok(Html(render_terminal_page(
        vm_id,
        &csrf_token,
        &state.config.upload_dir,
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
    let rootfs_path = build_user_rootfs_path(&state.config.user_rootfs_dir, db_user.id);
    info!("deleting saved rootfs");
    let _guard = timeout(Duration::from_secs(LOCK_TIMEOUT_SECS), state.rootfs_lock.lock())
        .await
        .context("timed out waiting for rootfs lock")?;
    let _ = tokio::fs::remove_file(&rootfs_path).await;
    drop(_guard);
    remove_user_vm(&state.vms, db_user.id)?;
    Ok(Redirect::to("/").into_response())
}

pub(crate) async fn get_terminal_page(
    user_vm: UserVmById,
    session: Session,
    State(state): State<AppState>,
) -> Result<Response, AppError> {
    build_terminal_response(&session, &state, user_vm.user_id, &user_vm.vm_id).await
}

pub(crate) async fn list_chat_sessions_handler(
    user_vm: UserVmById,
    State(state): State<AppState>,
) -> Result<Response, AppError> {
    Ok(list_chat_sessions(
        user_vm.guest_ip,
        &state.config.ssh_key_path,
        &state.config.ssh_user,
        &state.config.vm_host_key_path,
        Path::new(&state.config.ssh_user_home),
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
    user_vm: UserVmById,
    Query(query): Query<TranscriptQuery>,
    State(state): State<AppState>,
) -> Result<Response, AppError> {
    Ok(fetch_chat_history(
        user_vm.guest_ip,
        &state.config.ssh_key_path,
        &state.config.ssh_user,
        &state.config.vm_host_key_path,
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
    user_vm: UserVmById,
    session: Session,
    State(state): State<AppState>,
    Json(form): Json<DeleteChatSessionForm>,
) -> Result<Response, AppError> {
    if !validate_csrf(&session, &form.csrf_token).await {
        return Ok((StatusCode::FORBIDDEN, "Forbidden").into_response());
    }
    delete_chat_session(
        user_vm.guest_ip,
        &state.config.ssh_key_path,
        &state.config.ssh_user,
        &state.config.vm_host_key_path,
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
    user_vm: UserVmById,
    session: Session,
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<Response, AppError> {
    let chat_upload_metadata = extract_chat_upload_metadata(&mut multipart).await?;
    if !validate_csrf(&session, &chat_upload_metadata.csrf_token).await {
        return Ok((StatusCode::FORBIDDEN, "Forbidden").into_response());
    }
    info!("uploading chat attachment via sftp");
    let mut ssh_handle = connect_ssh(
        user_vm.guest_ip,
        &state.config.ssh_key_path,
        &state.config.ssh_user,
        &state.config.vm_host_key_path,
    )
    .await?;
    let sftp = open_sftp_session(&mut ssh_handle).await?;
    let remote_path = stream_chat_attachment(&mut multipart, &sftp).await?;
    let remote_path_str = remote_path
        .to_str()
        .context("remote path is not valid UTF-8")?;
    Ok(Json(serde_json::json!({"path": remote_path_str})).into_response())
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

async fn stream_chat_attachment(multipart: &mut Multipart, sftp: &SftpSession) -> Result<PathBuf> {
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

fn build_chat_upload_path(filename: &str) -> PathBuf {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is before Unix epoch")
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
    PathBuf::from("/tmp").join(format!("{ts}_{safe_name}"))
}

async fn write_chat_file_via_sftp(
    sftp: &SftpSession,
    path: &Path,
    reader: &mut (impl AsyncRead + Unpin),
) -> Result<()> {
    let path_str = path.to_str().context("chat upload path is not valid UTF-8")?;
    let mut file = sftp
        .create(path_str)
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
