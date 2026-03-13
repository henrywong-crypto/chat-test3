use anyhow::{Context, Result};
use russh_sftp::client::SftpSession;
use std::path::{Path, PathBuf};
use tokio::io::{copy, AsyncRead, AsyncWriteExt};

use common::validate_within_dir;

pub async fn write_file_via_sftp(
    sftp: SftpSession,
    remote_path: &str,
    upload_dir: &str,
    source: &mut (impl AsyncRead + Unpin),
) -> Result<()> {
    let resolved = resolve_upload_path(&sftp, remote_path, upload_dir).await?;
    let resolved_str = resolved.to_str().context("resolved path is not valid UTF-8")?;
    let mut file = sftp
        .create(resolved_str)
        .await
        .context("failed to create remote file")?;
    copy(source, &mut file)
        .await
        .context("failed to write file data")?;
    file.shutdown()
        .await
        .context("failed to close remote file")?;
    Ok(())
}

async fn resolve_upload_path(
    sftp: &SftpSession,
    remote_path: &str,
    upload_dir: &str,
) -> Result<PathBuf> {
    let path = Path::new(remote_path);
    let parent = path
        .parent()
        .and_then(|p| p.to_str())
        .context("upload path has no valid parent directory")?;
    let filename = path
        .file_name()
        .context("upload path has no filename")?;
    let canonical_parent = sftp
        .canonicalize(parent)
        .await
        .context("failed to resolve upload directory")?;
    let resolved = PathBuf::from(canonical_parent).join(filename);
    validate_within_dir(&resolved, Path::new(upload_dir))?;
    Ok(resolved)
}
