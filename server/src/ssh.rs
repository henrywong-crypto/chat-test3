use std::{
    path::PathBuf,
    sync::Arc,
    time::Duration,
};
use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use russh::{
    Channel,
    client::{self, Handle},
};
use russh_keys::key::PublicKey;
use russh_sftp::client::SftpSession;

pub(crate) struct SshClient {
    pub(crate) vm_host_key: Arc<PublicKey>,
}

#[async_trait]
impl client::Handler for SshClient {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &PublicKey,
    ) -> Result<bool, Self::Error> {
        Ok(server_public_key.fingerprint() == self.vm_host_key.fingerprint())
    }
}

pub(crate) async fn connect_ssh(
    guest_ip: &str,
    ssh_key_path: &PathBuf,
    ssh_user: &str,
    vm_host_key: &Arc<PublicKey>,
) -> Result<Handle<SshClient>> {
    let ssh_keypair = Arc::new(
        russh_keys::load_secret_key(ssh_key_path, None).context("failed to load SSH key")?,
    );
    let ssh_config = Arc::new(client::Config::default());
    let addr = format!("{guest_ip}:22");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    let mut ssh_handle = loop {
        let ssh_client = SshClient { vm_host_key: vm_host_key.clone() };
        match client::connect(ssh_config.clone(), addr.as_str(), ssh_client).await {
            Ok(ssh_handle) => break ssh_handle,
            Err(_) if tokio::time::Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            Err(connect_error) => bail!("SSH connect timed out: {connect_error}"),
        }
    };
    let auth_ok = ssh_handle.authenticate_publickey(ssh_user, ssh_keypair).await?;
    if !auth_ok {
        bail!("SSH authentication rejected");
    }
    Ok(ssh_handle)
}

pub(crate) async fn open_terminal_channel(
    ssh_handle: &mut Handle<SshClient>,
) -> Result<Channel<client::Msg>> {
    let ssh_channel = ssh_handle.channel_open_session().await?;
    ssh_channel.request_pty(false, "xterm-256color", 80, 24, 0, 0, &[]).await?;
    ssh_channel.exec(false, "tmux new-session -A -s main").await?;
    Ok(ssh_channel)
}

pub(crate) async fn open_sftp_session(
    ssh_handle: &mut Handle<SshClient>,
) -> Result<SftpSession> {
    let ssh_channel = ssh_handle.channel_open_session().await?;
    ssh_channel.request_subsystem(true, "sftp").await?;
    let sftp_session = SftpSession::new(ssh_channel.into_stream()).await?;
    Ok(sftp_session)
}
