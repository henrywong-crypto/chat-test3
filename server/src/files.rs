use anyhow::{anyhow, Context, Result};
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use russh_sftp::client::fs::ReadDir;
use serde::{Deserialize, Serialize};
use store::upsert_user;
use uuid::Uuid;

use crate::{
    auth::User,
    download::validate_within_dir,
    ssh::{connect_ssh, open_sftp_session},
    state::{find_vm_guest_ip_for_user, AppError, AppState},
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
    let guest_ip = find_vm_guest_ip_for_user(&state.vms, &vm_id, db_user.id)
        .ok_or_else(|| anyhow!("Session {vm_id} not found"))?;
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
    let read_dir = sftp
        .read_dir(&real_path)
        .await
        .context("failed to read remote directory")?;
    let entries = collect_entries(read_dir);
    Ok(Json(ListResponse { entries }).into_response())
}

fn collect_entries(read_dir: ReadDir) -> Vec<FileEntry> {
    let mut dirs: Vec<FileEntry> = Vec::new();
    let mut files: Vec<FileEntry> = Vec::new();
    for entry in read_dir {
        let name = entry.file_name();
        let is_dir = entry.file_type().is_dir();
        let size = entry.metadata().size.unwrap_or(0);
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
    dirs
}
