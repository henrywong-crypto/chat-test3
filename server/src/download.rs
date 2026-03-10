use anyhow::{bail, Context, Result};
use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{header, HeaderValue, Response, StatusCode},
    response::IntoResponse,
};
use bytes::Bytes;
use futures::Stream;
use russh::client::Handle;
use russh_sftp::client::{fs::File as SftpFile, SftpSession};
use serde::Deserialize;
use std::{
    io,
    pin::Pin,
    task::{Context as TaskContext, Poll},
};
use store::upsert_user;
use tokio_util::io::ReaderStream;
use uuid::Uuid;
use zip::write::SimpleFileOptions;

use crate::{
    auth::User,
    ssh::{connect_ssh, open_sftp_session, SshClient},
    state::{find_vm_guest_ip_for_user, AppError, AppState},
};

const MAX_DOWNLOAD_BYTES: usize = 100 * 1024 * 1024; // 100 MB

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
    let real_path = sftp
        .canonicalize(&query.path)
        .await
        .context("failed to resolve remote path")?;
    validate_within_dir(&real_path, &state.upload_dir)?;
    let metadata = sftp
        .symlink_metadata(&real_path)
        .await
        .context("failed to stat remote path")?;
    if metadata.is_dir() {
        let dirname = extract_filename(&real_path).to_string();
        let zip_data = build_directory_zip(&sftp, &real_path, &state.upload_dir).await?;
        build_zip_response(zip_data, &format!("{dirname}.zip"))
    } else {
        let filename = extract_filename(&real_path).to_string();
        build_streaming_file_response(ssh_handle, sftp, &real_path, &filename).await
    }
}

async fn build_streaming_file_response(
    ssh_handle: Handle<SshClient>,
    sftp: SftpSession,
    path: &str,
    filename: &str,
) -> Result<Response<Body>, AppError> {
    let file = sftp
        .open(path)
        .await
        .context("failed to open remote file")?;
    let stream = SftpFileStream {
        inner: ReaderStream::new(file),
        _sftp: sftp,
        _ssh_handle: ssh_handle,
    };
    let content_disposition =
        HeaderValue::from_str(&format!("attachment; filename=\"{filename}\""))
            .unwrap_or_else(|_| HeaderValue::from_static("attachment"));
    let response = Response::builder()
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(header::CONTENT_DISPOSITION, content_disposition)
        .body(Body::from_stream(stream))
        .context("failed to build response")?;
    Ok(response)
}

struct SftpFileStream {
    inner: ReaderStream<SftpFile>,
    _sftp: SftpSession,
    _ssh_handle: Handle<SshClient>,
}

impl Stream for SftpFileStream {
    type Item = Result<Bytes, io::Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.inner).poll_next(cx)
    }
}

fn extract_filename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or("download")
}

const MAX_ZIP_DEPTH: usize = 10;

async fn build_directory_zip(
    sftp: &SftpSession,
    dir_path: &str,
    upload_dir: &str,
) -> Result<Vec<u8>> {
    let mut cursor = std::io::Cursor::new(Vec::new());
    let mut zip = zip::ZipWriter::new(&mut cursor);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let mut total_bytes: usize = 0;
    let mut dirs_to_visit: Vec<(String, usize)> = vec![(dir_path.to_string(), 0)];
    while let Some((dir, depth)) = dirs_to_visit.pop() {
        let read_dir = sftp
            .read_dir(&dir)
            .await
            .context("failed to read remote directory")?;
        for entry in read_dir {
            let name = entry.file_name();
            if name == "." || name == ".." {
                continue;
            }
            let child_path = format!("{}/{}", dir.trim_end_matches('/'), name);
            let file_type = entry.file_type();
            if file_type.is_symlink() {
                continue;
            }
            // child_path is already canonical: dir_path was canonicalized by the caller,
            // name is a bare filename from read_dir (no path separators), and symlinks are skipped.
            validate_within_dir(&child_path, upload_dir)?;
            if file_type.is_dir() {
                if depth + 1 < MAX_ZIP_DEPTH {
                    dirs_to_visit.push((child_path, depth + 1));
                }
            } else {
                let relative = child_path
                    .strip_prefix(dir_path)
                    .unwrap_or(&child_path)
                    .trim_start_matches('/');
                let data = read_file_buffered(sftp, &child_path).await?;
                total_bytes += data.len();
                if total_bytes > MAX_DOWNLOAD_BYTES {
                    bail!("directory exceeds maximum download size of 100 MB");
                }
                zip.start_file(relative, options)
                    .context("failed to add file to zip")?;
                std::io::Write::write_all(&mut zip, &data)
                    .context("failed to write file data to zip")?;
            }
        }
    }
    zip.finish().context("failed to finalize zip")?;
    Ok(cursor.into_inner())
}

async fn read_file_buffered(sftp: &SftpSession, path: &str) -> Result<Vec<u8>> {
    use tokio::io::AsyncReadExt;
    let mut file = sftp
        .open(path)
        .await
        .context("failed to open remote file")?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf)
        .await
        .context("failed to read remote file")?;
    Ok(buf)
}

pub(crate) fn validate_within_dir(real_path: &str, allowed_dir: &str) -> Result<()> {
    let allowed_dir = allowed_dir.trim_end_matches('/');
    if !real_path.starts_with(allowed_dir)
        || (real_path.len() > allowed_dir.len() && !real_path[allowed_dir.len()..].starts_with('/'))
    {
        bail!("path is outside the allowed directory");
    }
    Ok(())
}

fn build_zip_response(data: Vec<u8>, filename: &str) -> Result<Response<Body>, AppError> {
    build_buffered_response(data, "application/zip", filename)
}

fn build_buffered_response(
    data: Vec<u8>,
    content_type: &str,
    filename: &str,
) -> Result<Response<Body>, AppError> {
    let content_disposition =
        HeaderValue::from_str(&format!("attachment; filename=\"{filename}\""))
            .unwrap_or_else(|_| HeaderValue::from_static("attachment"));
    let response = Response::builder()
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CONTENT_DISPOSITION, content_disposition)
        .body(Body::from(data))
        .context("failed to build response")?;
    Ok(response)
}
