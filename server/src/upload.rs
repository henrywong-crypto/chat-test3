use anyhow::{Context, Result, anyhow};
use axum::{
    extract::{Multipart, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use common::validate_within_dir;
use futures::TryStreamExt;
use russh_sftp::client::SftpSession;
use sftp_client::open_sftp_session;
use ssh_client::connect_ssh;
use std::{
    io::{Error as IoError, ErrorKind},
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::time::timeout;
use tokio_util::io::StreamReader;
use tower_sessions::Session;

use crate::{
    handlers::{UserVm, attach_csrf_token, validate_csrf},
    state::{AppError, AppState},
};

const SFTP_OP_TIMEOUT_SECS: u64 = 30;

struct UploadMetadata {
    csrf_token: String,
    remote_path: String,
}

pub(crate) async fn upload_file_handler(
    user_vm: UserVm,
    session: Session,
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<Response, AppError> {
    let upload_metadata = extract_upload_metadata(&mut multipart).await?;
    let Some(csrf_token) = validate_csrf(&session, &upload_metadata.csrf_token).await? else {
        return Ok((StatusCode::FORBIDDEN, "Forbidden").into_response());
    };
    let mut ssh_handle = connect_ssh(
        user_vm.guest_ip,
        &state.config.ssh_key_path,
        &state.config.ssh_user,
        &state.config.vm_host_key_path,
    )
    .await?;
    let sftp = open_sftp_session(&mut ssh_handle).await?;
    let real_path = PathBuf::from(
        timeout(
            Duration::from_secs(SFTP_OP_TIMEOUT_SECS),
            sftp.canonicalize(&upload_metadata.remote_path),
        )
        .await
        .context("canonicalize timed out")?
        .context("failed to resolve remote path")?,
    );
    validate_within_dir(&real_path, &state.config.upload_dir)?;
    stream_upload_file(
        &mut multipart,
        &sftp,
        &real_path,
        &state.config.upload_dir,
    )
    .await?;
    Ok(attach_csrf_token((StatusCode::OK, "").into_response(), &csrf_token))
}

async fn extract_upload_metadata(multipart: &mut Multipart) -> Result<UploadMetadata> {
    let mut csrf_token: Option<String> = None;
    let mut remote_path: Option<String> = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .context("failed to read multipart field")?
    {
        let name = field.name().context("multipart field missing name")?.to_owned();
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
    Ok(UploadMetadata {
        csrf_token,
        remote_path,
    })
}

async fn stream_upload_file(
    multipart: &mut Multipart,
    sftp: &SftpSession,
    remote_path: &Path,
    upload_dir: &Path,
) -> Result<()> {
    while let Some(field) = multipart
        .next_field()
        .await
        .context("failed to read multipart field")?
    {
        if field.name().context("multipart field missing name")? == "file" {
            let mut reader =
                StreamReader::new(field.map_err(|e| IoError::new(ErrorKind::Other, e)));
            return upload::write_file_via_sftp(sftp, remote_path, upload_dir, &mut reader).await;
        }
    }
    Err(anyhow!("missing 'file' field"))
}
