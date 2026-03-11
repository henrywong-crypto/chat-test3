use anyhow::{anyhow, Context, Result};
use axum::{
    extract::{Multipart, Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use bytes::Bytes;
use ssh_client::{connect_ssh, open_sftp_session};
use store::upsert_user;
use tower_sessions::Session;
use uuid::Uuid;

use crate::{
    auth::User,
    state::{find_vm_guest_ip_for_user, AppError, AppState},
};

pub(crate) async fn upload_file_handler(
    user: User,
    session: Session,
    Path(vm_id): Path<String>,
    State(state): State<AppState>,
    multipart: Multipart,
) -> Result<Response, AppError> {
    if Uuid::parse_str(&vm_id).is_err() {
        return Ok((StatusCode::NOT_FOUND, "Not found").into_response());
    }
    let db_user = upsert_user(&state.db, &user.email).await?;
    let (csrf_token, remote_path, data) = extract_upload_fields(multipart).await?;
    if !validate_csrf(&session, &csrf_token).await {
        return Ok((StatusCode::FORBIDDEN, "Forbidden").into_response());
    }
    let guest_ip = match find_vm_guest_ip_for_user(&state.vms, &vm_id, db_user.id) {
        Some(ip) => ip,
        None => return Ok((StatusCode::NOT_FOUND, "Session not found or expired").into_response()),
    };
    let mut ssh_handle = connect_ssh(
        &guest_ip,
        &state.ssh_key_path,
        &state.ssh_user,
        &state.vm_host_key_path,
    )
    .await?;
    let sftp = open_sftp_session(&mut ssh_handle).await?;
    upload::write_file_via_sftp(sftp, &remote_path, &state.upload_dir, &data).await?;
    Ok((StatusCode::OK, "").into_response())
}

async fn validate_csrf(session: &Session, submitted: &str) -> bool {
    match session.get::<String>("csrf_token").await {
        Ok(Some(token)) => token == submitted,
        _ => false,
    }
}

async fn extract_upload_fields(mut multipart: Multipart) -> Result<(String, String, Bytes)> {
    let mut csrf_token: Option<String> = None;
    let mut remote_path: Option<String> = None;
    let mut file_data: Option<Bytes> = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .context("failed to read multipart field")?
    {
        match field.name() {
            Some("csrf_token") => {
                csrf_token = Some(
                    field
                        .text()
                        .await
                        .context("failed to read csrf_token field")?,
                );
            }
            Some("path") => {
                remote_path = Some(field.text().await.context("failed to read path field")?);
            }
            Some("file") => {
                file_data = Some(field.bytes().await.context("failed to read file field")?);
            }
            _ => {}
        }
    }
    let csrf_token = csrf_token.ok_or_else(|| anyhow!("missing 'csrf_token' field"))?;
    let remote_path = remote_path.ok_or_else(|| anyhow!("missing 'path' field"))?;
    let file_data = file_data.ok_or_else(|| anyhow!("missing 'file' field"))?;
    Ok((csrf_token, remote_path, file_data))
}
