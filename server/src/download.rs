use anyhow::{anyhow, bail, Context, Result};
use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{header, HeaderValue, Response, StatusCode},
    response::IntoResponse,
};
use serde::Deserialize;
use store::upsert_user;
use tokio::io::AsyncReadExt;
use uuid::Uuid;

use crate::{
    auth::User,
    ssh::{connect_ssh, open_sftp_session, SshClient},
    state::{find_vm_guest_ip_for_user, AppError, AppState},
};

#[derive(Deserialize)]
pub(crate) struct DownloadQuery {
    path: String,
}

pub(crate) async fn download_file_handler(
    user: User,
    Path(vm_id): Path<String>,
    Query(query): Query<DownloadQuery>,
    State(state): State<AppState>,
) -> Result<Response<Body>, AppError> {
    if Uuid::parse_str(&vm_id).is_err() {
        return Ok((StatusCode::NOT_FOUND, "Not found").into_response());
    }
    let db_user = upsert_user(&state.db, &user.email).await?;
    let guest_ip = find_vm_guest_ip_for_user(&state.vms, &vm_id, db_user.id)
        .ok_or_else(|| anyhow!("Session {vm_id} not found"))?;
    let validated_path = validate_download_path(&query.path, &state.upload_dir)?;
    let filename = extract_filename(&validated_path).to_string();
    let mut ssh_handle = connect_ssh(
        &guest_ip,
        &state.ssh_key_path,
        &state.ssh_user,
        &state.vm_host_key_path,
    )
    .await?;
    let data = read_file_via_sftp(&mut ssh_handle, &validated_path).await?;
    build_download_response(data, &filename)
}

fn validate_download_path(remote_path: &str, download_dir: &str) -> Result<String> {
    let mut components: Vec<&str> = Vec::new();
    for part in remote_path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                components.pop();
            }
            c => components.push(c),
        }
    }
    let normalized = format!("/{}", components.join("/"));
    let download_dir = download_dir.trim_end_matches('/');
    if !normalized.starts_with(download_dir)
        || (normalized.len() > download_dir.len()
            && !normalized[download_dir.len()..].starts_with('/'))
    {
        bail!("download path {remote_path:?} is outside the allowed directory");
    }
    Ok(normalized)
}

fn extract_filename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or("download")
}

async fn read_file_via_sftp(
    ssh_handle: &mut russh::client::Handle<SshClient>,
    remote_path: &str,
) -> Result<Vec<u8>> {
    let sftp_session = open_sftp_session(ssh_handle).await?;
    let mut file = sftp_session
        .open(remote_path)
        .await
        .context("failed to open remote file")?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf)
        .await
        .context("failed to read remote file")?;
    Ok(buf)
}

fn build_download_response(data: Vec<u8>, filename: &str) -> Result<Response<Body>, AppError> {
    let content_disposition = HeaderValue::from_str(&format!("attachment; filename=\"{filename}\""))
        .unwrap_or_else(|_| HeaderValue::from_static("attachment"));
    let response = Response::builder()
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(header::CONTENT_DISPOSITION, content_disposition)
        .body(Body::from(data))
        .context("failed to build download response")?;
    Ok(response)
}
