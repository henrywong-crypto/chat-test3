use anyhow::{anyhow, Context, Result};
use axum::{
    extract::{Multipart, Path as AxumPath, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use futures::TryStreamExt;
use russh_sftp::client::SftpSession;
use sftp_client::open_sftp_session;
use ssh_client::connect_ssh;
use std::{
    io::{Error as IoError, ErrorKind},
    path::Path,
};
use store::upsert_user;
use tokio_util::io::StreamReader;
use tower_sessions::Session;
use uuid::Uuid;

use crate::{
    auth::User,
    state::{find_vm_guest_ip_for_user, AppError, AppState},
};

struct UploadMetadata {
    csrf_token: String,
    remote_path: String,
}

pub(crate) async fn upload_file_handler(
    user: User,
    session: Session,
    AxumPath(vm_id): AxumPath<String>,
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<Response, AppError> {
    if Uuid::parse_str(&vm_id).is_err() {
        return Ok((StatusCode::NOT_FOUND, "Not found").into_response());
    }
    let db_user = upsert_user(&state.db, &user.email).await?;
    let upload_metadata = extract_upload_metadata(&mut multipart).await?;
    if !validate_csrf(&session, &upload_metadata.csrf_token).await {
        return Ok((StatusCode::FORBIDDEN, "Forbidden").into_response());
    }
    let Some(guest_ip) = find_vm_guest_ip_for_user(&state.vms, &vm_id, db_user.id)? else {
        return Ok((StatusCode::NOT_FOUND, "Session not found or expired").into_response());
    };
    let mut ssh_handle = connect_ssh(
        &guest_ip,
        &state.ssh_key_path,
        &state.ssh_user,
        &state.vm_host_key_path,
    )
    .await?;
    let sftp = open_sftp_session(&mut ssh_handle).await?;
    stream_upload_file(&mut multipart, sftp, Path::new(&upload_metadata.remote_path), Path::new(&state.upload_dir)).await?;
    Ok((StatusCode::OK, "").into_response())
}

async fn validate_csrf(session: &Session, submitted: &str) -> bool {
    session
        .get::<String>("csrf_token")
        .await
        .ok()
        .flatten()
        .is_some_and(|token| token == submitted)
}

async fn extract_upload_metadata(multipart: &mut Multipart) -> Result<UploadMetadata> {
    let mut csrf_token: Option<String> = None;
    let mut remote_path: Option<String> = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .context("failed to read multipart field")?
    {
        let name = field.name().unwrap_or("").to_owned();
        if name == "csrf_token" {
            csrf_token = Some(
                field
                    .text()
                    .await
                    .context("failed to read csrf_token field")?,
            );
        } else if name == "path" {
            remote_path = Some(field.text().await.context("failed to read path field")?);
        }
        if csrf_token.is_some() && remote_path.is_some() {
            break;
        }
    }
    let csrf_token = csrf_token.ok_or_else(|| anyhow!("missing 'csrf_token' field"))?;
    let remote_path = remote_path.ok_or_else(|| anyhow!("missing 'path' field"))?;
    Ok(UploadMetadata { csrf_token, remote_path })
}

async fn stream_upload_file(
    multipart: &mut Multipart,
    sftp: SftpSession,
    remote_path: &Path,
    upload_dir: &Path,
) -> Result<()> {
    while let Some(field) = multipart
        .next_field()
        .await
        .context("failed to read multipart field")?
    {
        if field.name().unwrap_or("") == "file" {
            let mut reader =
                StreamReader::new(field.map_err(|e| IoError::new(ErrorKind::Other, e)));
            return upload::write_file_via_sftp(sftp, remote_path, upload_dir, &mut reader).await;
        }
    }
    Err(anyhow!("missing 'file' field"))
}
