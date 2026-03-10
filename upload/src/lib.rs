use anyhow::{anyhow, bail, Context, Result};
use russh_sftp::client::SftpSession;
use std::path::Path;
use tokio::io::AsyncWriteExt;

use download::validate_within_dir;

pub async fn write_file_via_sftp(
    sftp: SftpSession,
    remote_path: &str,
    upload_dir: &str,
    data: &[u8],
) -> Result<()> {
    let resolved_path = resolve_upload_path(&sftp, remote_path, upload_dir).await?;
    let mut file = sftp
        .create(&resolved_path)
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

async fn resolve_upload_path(sftp: &SftpSession, remote_path: &str, upload_dir: &str) -> Result<String> {
    let path = Path::new(remote_path);
    let parent = path.parent().and_then(|p| p.to_str()).unwrap_or(".");
    let filename = path
        .file_name()
        .and_then(|f| f.to_str())
        .ok_or_else(|| anyhow!("upload path has no filename"))?;
    if filename == ".." {
        bail!("upload path has no filename");
    }
    let canonical_parent = sftp
        .canonicalize(parent)
        .await
        .context("failed to resolve upload directory")?;
    let resolved = format!("{}/{}", canonical_parent.trim_end_matches('/'), filename);
    validate_within_dir(&resolved, upload_dir)?;
    Ok(resolved)
}
