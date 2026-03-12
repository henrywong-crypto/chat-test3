use anyhow::Context;
use axum::{
    extract::{Path, Query, State},
    http::Response,
    http::StatusCode,
    response::IntoResponse,
};
use download::{
    file::build_streaming_file_response,
    validate_within_dir,
    zip::build_streaming_zip_response,
};
use serde::Deserialize;
use sftp_client::open_sftp_session;
use ssh_client::connect_ssh;
use store::upsert_user;
use uuid::Uuid;

use crate::{
    auth::User,
    state::{find_user_vm_guest_ip, find_vm_guest_ip_for_user, AppError, AppState},
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
) -> Result<Response<axum::body::Body>, AppError> {
    if Uuid::parse_str(&vm_id).is_err() {
        return Ok((StatusCode::NOT_FOUND, "Not found").into_response());
    }
    let db_user = upsert_user(&state.db, &user.email).await?;
    let guest_ip = match find_vm_guest_ip_for_user(&state.vms, &vm_id, db_user.id)
        .or_else(|| find_user_vm_guest_ip(&state.vms, db_user.id))
    {
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
        let dirname = real_path.rsplit('/').next().unwrap_or("download").to_owned();
        Ok(build_streaming_zip_response(
            sftp,
            real_path,
            state.upload_dir.clone(),
            &format!("{dirname}.zip"),
        ))
    } else {
        Ok(build_streaming_file_response(sftp, std::path::Path::new(&real_path))
            .await
            .context("failed to build file response")?)
    }
}
