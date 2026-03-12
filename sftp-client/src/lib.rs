pub use russh_sftp::client::{fs::DirEntry, SftpSession};

use anyhow::Result;
use russh::client;
use ssh_client::SshClient;

pub async fn open_sftp_session(ssh_handle: &mut client::Handle<SshClient>) -> Result<SftpSession> {
    let ssh_channel = ssh_handle.channel_open_session().await?;
    ssh_channel.request_subsystem(true, "sftp").await?;
    let sftp_session = SftpSession::new(ssh_channel.into_stream()).await?;
    Ok(sftp_session)
}
