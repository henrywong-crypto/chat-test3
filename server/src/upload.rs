use anyhow::{anyhow, bail, Context, Result};
use axum::{
    extract::{Multipart, Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use bytes::Bytes;
use russh::client::Handle;
use tokio::io::AsyncWriteExt;
use tower_sessions::Session;
use uuid::Uuid;

use crate::{
    auth::User,
    ssh::{connect_ssh, open_sftp_session, SshClient},
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
    let (csrf_token, remote_path, data) = extract_upload_fields(multipart).await?;
    if !validate_csrf(&session, &csrf_token).await {
        return Ok((StatusCode::FORBIDDEN, "Forbidden").into_response());
    }
    let guest_ip = find_vm_guest_ip_for_user(&state.vms, &vm_id, &user.email)
        .ok_or_else(|| anyhow!("Session {vm_id} not found"))?;
    let validated_path = validate_upload_path(&remote_path, &state.upload_dir)?;
    let mut ssh_handle = connect_ssh(
        &guest_ip,
        &state.ssh_key_path,
        &state.ssh_user,
        &state.vm_host_key_path,
    )
    .await?;
    write_file_via_sftp(&mut ssh_handle, &validated_path, &data).await?;
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

fn validate_upload_path(remote_path: &str, upload_dir: &str) -> Result<String> {
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
    let upload_dir = upload_dir.trim_end_matches('/');
    if !normalized.starts_with(upload_dir)
        || (normalized.len() > upload_dir.len() && !normalized[upload_dir.len()..].starts_with('/'))
    {
        bail!("upload path {remote_path:?} is outside the upload directory");
    }
    Ok(normalized)
}

async fn write_file_via_sftp(
    ssh_handle: &mut Handle<SshClient>,
    remote_path: &str,
    data: &[u8],
) -> Result<()> {
    let sftp_session = open_sftp_session(ssh_handle).await?;
    let mut file = sftp_session
        .create(remote_path)
        .await
        .context("failed to create remote file")?;
    file.write_all(data)
        .await
        .context("failed to write file data")?;
    file.shutdown()
        .await
        .context("failed to close remote file")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::validate_upload_path;

    #[test]
    fn accepts_path_inside_upload_dir() {
        let result = validate_upload_path("/home/ubuntu/file.txt", "/home/ubuntu");
        assert_eq!(result.unwrap(), "/home/ubuntu/file.txt");
    }

    #[test]
    fn accepts_path_with_subdirectory() {
        let result = validate_upload_path("/home/ubuntu/sub/file.txt", "/home/ubuntu");
        assert_eq!(result.unwrap(), "/home/ubuntu/sub/file.txt");
    }

    #[test]
    fn rejects_dotdot_escape() {
        let result = validate_upload_path("/home/ubuntu/../etc/passwd", "/home/ubuntu");
        assert!(result.is_err());
    }

    #[test]
    fn rejects_path_outside_upload_dir() {
        let result = validate_upload_path("/etc/passwd", "/home/ubuntu");
        assert!(result.is_err());
    }

    #[test]
    fn rejects_prefix_confusion() {
        let result = validate_upload_path("/home/ubuntu2/file.txt", "/home/ubuntu");
        assert!(result.is_err());
    }

    #[test]
    fn normalizes_dotdot_within_allowed() {
        let result = validate_upload_path("/home/ubuntu/sub/../file.txt", "/home/ubuntu");
        assert_eq!(result.unwrap(), "/home/ubuntu/file.txt");
    }
}
