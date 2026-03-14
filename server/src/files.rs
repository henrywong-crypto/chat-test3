use anyhow::{Context, Result};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use common::validate_within_dir;
use russh_sftp::client::{SftpSession, fs::DirEntry};
use serde::{Deserialize, Serialize};
use sftp_client::open_sftp_session;
use ssh_client::connect_ssh;
use std::path::{Path as StdPath, PathBuf};
use store::upsert_user;
use uuid::Uuid;

use crate::{
    auth::User,
    state::{AppError, AppState, find_user_vm_guest_ip, find_vm_guest_ip_for_user},
};

#[derive(Deserialize)]
pub(crate) struct ListQuery {
    path: String,
}

#[derive(Serialize)]
struct FileEntry {
    name: String,
    is_dir: bool,
    size: u64,
}

#[derive(Serialize)]
struct ListResponse {
    entries: Vec<FileEntry>,
}

pub(crate) async fn list_files_handler(
    user: User,
    Path(vm_id): Path<String>,
    Query(query): Query<ListQuery>,
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    if Uuid::parse_str(&vm_id).is_err() {
        return Ok((StatusCode::NOT_FOUND, "Not found").into_response());
    }
    let db_user = upsert_user(&state.db, &user.email).await?;
    let guest_ip = match find_vm_guest_ip_for_user(&state.vms, &vm_id, db_user.id)? {
        Some(ip) => ip,
        None => match find_user_vm_guest_ip(&state.vms, db_user.id)? {
            Some(ip) => ip,
            None => {
                return Ok((StatusCode::NOT_FOUND, "Session not found or expired").into_response());
            }
        },
    };
    let mut ssh_handle = connect_ssh(
        &guest_ip,
        &state.ssh_key_path,
        &state.ssh_user,
        &state.vm_host_key_path,
    )
    .await?;
    let sftp = open_sftp_session(&mut ssh_handle).await?;
    let real_path = PathBuf::from(
        sftp.canonicalize(&query.path)
            .await
            .context("failed to resolve remote path")?,
    );
    validate_within_dir(&real_path, &PathBuf::from(&state.upload_dir))?;
    let real_path_str = real_path
        .to_str()
        .context("resolved path is not valid UTF-8")?
        .to_owned();
    let read_dir = sftp
        .read_dir(&real_path_str)
        .await
        .context("failed to read remote directory")?;
    let entries = collect_file_entries(&sftp, &real_path, read_dir.collect()).await?;
    Ok(Json(ListResponse { entries }).into_response())
}

async fn collect_file_entries(
    sftp: &SftpSession,
    dir_path: &StdPath,
    raw_entries: Vec<DirEntry>,
) -> Result<Vec<FileEntry>> {
    let mut dirs: Vec<FileEntry> = Vec::new();
    let mut files: Vec<FileEntry> = Vec::new();
    for entry in raw_entries {
        let name = entry.file_name();
        if name == "." || name == ".." {
            continue;
        }
        let metadata = entry.metadata();
        let is_dir = if metadata.permissions.is_some() {
            metadata.file_type().is_dir()
        } else {
            let child_path = dir_path.join(&name);
            let child_str = child_path
                .to_str()
                .context("child path is not valid UTF-8")?;
            sftp.symlink_metadata(child_str)
                .await
                .map(|m| m.file_type().is_dir())
                .context("failed to stat directory entry")?
        };
        let size = metadata.size.context("missing file size")?;
        let file_entry = FileEntry { name, is_dir, size };
        if is_dir {
            dirs.push(file_entry);
        } else {
            files.push(file_entry);
        }
    }
    dirs.sort_by(|a, b| a.name.cmp(&b.name));
    files.sort_by(|a, b| a.name.cmp(&b.name));
    dirs.extend(files);
    Ok(dirs)
}
