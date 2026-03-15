use anyhow::Context;
use axum::{
    extract::{Path, Query, State},
    http::{Response, StatusCode},
    response::IntoResponse,
};
use common::validate_within_dir;
use download::{file::build_streaming_file_response, zip::build_streaming_zip_response};
use serde::Deserialize;
use sftp_client::open_sftp_session;
use ssh_client::connect_ssh;
use std::path::PathBuf;
use store::upsert_user;
use uuid::Uuid;

use crate::{
    auth::User,
    state::{AppError, AppState, find_vm_guest_ip_for_user},
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
    let Some(guest_ip) = find_vm_guest_ip_for_user(&state.vms, &vm_id, db_user.id)? else {
        return Ok((StatusCode::NOT_FOUND, "Session not found or expired").into_response());
    };
    let mut ssh_handle = connect_ssh(
        guest_ip,
        &state.config.ssh_key_path,
        &state.config.ssh_user,
        &state.config.vm_host_key_path,
    )
    .await?;
    let sftp = open_sftp_session(&mut ssh_handle).await?;
    let real_path = PathBuf::from(
        sftp.canonicalize(&query.path)
            .await
            .context("failed to resolve remote path")?,
    );
    let upload_dir = PathBuf::from(&state.config.upload_dir);
    validate_within_dir(&real_path, &upload_dir)?;
    let real_path_str = real_path
        .to_str()
        .context("resolved path is not valid UTF-8")?
        .to_owned();
    let metadata = sftp
        .symlink_metadata(&real_path_str)
        .await
        .context("failed to stat remote path")?;
    if metadata.is_dir() {
        let dirname = real_path
            .file_name()
            .and_then(|f| f.to_str())
            .context("path has no final component")?
            .to_owned();
        Ok(build_streaming_zip_response(
            sftp,
            &real_path,
            &upload_dir,
            &format!("{dirname}.zip"),
        )?)
    } else {
        Ok(build_streaming_file_response(sftp, &real_path)
            .await
            .context("failed to build file response")?)
    }
}
