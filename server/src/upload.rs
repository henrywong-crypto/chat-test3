use anyhow::{anyhow, bail, Context, Result};
use axum::{
    extract::{Multipart, Path, State},
    http::StatusCode,
};
use bytes::Bytes;
use russh::client::Handle;
use tokio::io::AsyncWriteExt;

use crate::{
    auth::User,
    ssh::{connect_ssh, open_sftp_session, SshClient},
    state::{AppError, AppState, find_vm_guest_ip},
};

pub(crate) async fn upload_file_handler(
    _user: User,
    Path(vm_id): Path<String>,
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<StatusCode, AppError> {
    let guest_ip = find_vm_guest_ip(&state.vms, &vm_id)
        .ok_or_else(|| anyhow!("VM {vm_id} not found"))?;
    let (remote_path, data) = extract_upload_fields(&mut multipart).await?;
    let validated_path = validate_upload_path(&remote_path, &state.upload_dir)?;
    let mut ssh_handle =
        connect_ssh(&guest_ip, &state.ssh_key_path, &state.ssh_user, &state.vm_host_key_path).await?;
    write_file_via_sftp(&mut ssh_handle, &validated_path, &data).await?;
    Ok(StatusCode::OK)
}

async fn extract_upload_fields(multipart: &mut Multipart) -> Result<(String, Bytes)> {
    let mut remote_path: Option<String> = None;
    let mut file_data: Option<Bytes> = None;
    while let Some(field) = multipart.next_field().await.context("failed to read multipart field")? {
        match field.name() {
            Some("path") => {
                remote_path = Some(field.text().await.context("failed to read path field")?);
            }
            Some("file") => {
                file_data = Some(field.bytes().await.context("failed to read file field")?);
            }
            _ => {}
        }
    }
    let remote_path = remote_path.ok_or_else(|| anyhow!("missing 'path' field"))?;
    let file_data = file_data.ok_or_else(|| anyhow!("missing 'file' field"))?;
    Ok((remote_path, file_data))
}

fn validate_upload_path(remote_path: &str, upload_dir: &str) -> Result<String> {
    let mut components: Vec<&str> = Vec::new();
    for part in remote_path.split('/') {
        match part {
            "" | "." => {}
            ".." => { components.pop(); }
            c => components.push(c),
        }
    }
    let normalized = format!("/{}", components.join("/"));
    let upload_dir = upload_dir.trim_end_matches('/');
    if !normalized.starts_with(upload_dir)
        || (normalized.len() > upload_dir.len()
            && !normalized[upload_dir.len()..].starts_with('/'))
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
    let sftp = open_sftp_session(ssh_handle).await?;
    let mut file = sftp.create(remote_path).await.context("failed to create remote file")?;
    file.write_all(data).await.context("failed to write file data")?;
    file.shutdown().await.context("failed to close remote file")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::validate_upload_path;

    #[test]
    fn accepts_path_inside_upload_dir() {
        let result = validate_upload_path("/home/user/uploads/file.txt", "/home/user/uploads");
        assert_eq!(result.unwrap(), "/home/user/uploads/file.txt");
    }

    #[test]
    fn accepts_path_with_subdirectory() {
        let result = validate_upload_path("/home/user/uploads/sub/file.txt", "/home/user/uploads");
        assert_eq!(result.unwrap(), "/home/user/uploads/sub/file.txt");
    }

    #[test]
    fn rejects_dotdot_escape() {
        let result = validate_upload_path("/home/user/uploads/../etc/passwd", "/home/user/uploads");
        assert!(result.is_err());
    }

    #[test]
    fn rejects_path_outside_upload_dir() {
        let result = validate_upload_path("/etc/passwd", "/home/user/uploads");
        assert!(result.is_err());
    }

    #[test]
    fn rejects_prefix_confusion() {
        let result = validate_upload_path("/home/user/uploads2/file.txt", "/home/user/uploads");
        assert!(result.is_err());
    }

    #[test]
    fn normalizes_dotdot_within_allowed() {
        let result =
            validate_upload_path("/home/user/uploads/sub/../file.txt", "/home/user/uploads");
        assert_eq!(result.unwrap(), "/home/user/uploads/file.txt");
    }
}
