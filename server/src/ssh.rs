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
use russh_keys::{key::PublicKey, load_public_key, load_secret_key};
use russh_sftp::client::SftpSession;

pub(crate) struct SshClient {
    vm_host_key: Option<PublicKey>,
}

#[async_trait]
impl client::Handler for SshClient {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &PublicKey,
    ) -> Result<bool, Self::Error> {
        let accepted = match &self.vm_host_key {
            Some(key) => server_public_key.fingerprint() == key.fingerprint(),
            None => true,
        };
        eprintln!("[ssh] host key check: fingerprint={} accepted={accepted}", server_public_key.fingerprint());
        Ok(accepted)
    }
}

pub(crate) async fn connect_ssh(
    guest_ip: &str,
    ssh_key_path: &PathBuf,
    ssh_user: &str,
    vm_host_key_path: &PathBuf,
) -> Result<Handle<SshClient>> {
    eprintln!("[ssh] connecting to {guest_ip}:22 user={ssh_user} key={} host_key_file={}", ssh_key_path.display(), vm_host_key_path.display());
    let vm_host_key = load_public_key(vm_host_key_path).ok();
    eprintln!("[ssh] vm host key loaded: {}", vm_host_key.is_some());
    let ssh_keypair = Arc::new(
        load_secret_key(ssh_key_path, None).context("failed to load SSH key")?,
    );
    let ssh_config = Arc::new(client::Config::default());
    let addr = format!("{guest_ip}:22");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    let mut attempt = 0u32;
    let mut ssh_handle = loop {
        attempt += 1;
        let ssh_client = SshClient { vm_host_key: vm_host_key.clone() };
        match client::connect(ssh_config.clone(), addr.as_str(), ssh_client).await {
            Ok(ssh_handle) => {
                eprintln!("[ssh] connected on attempt {attempt}");
                break ssh_handle;
            }
            Err(e) if tokio::time::Instant::now() < deadline => {
                eprintln!("[ssh] attempt {attempt} failed: {e} — retrying");
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            Err(connect_error) => bail!("SSH connect timed out: {connect_error}"),
        }
    };
    eprintln!("[ssh] authenticating as {ssh_user}");
    let auth_ok = ssh_handle.authenticate_publickey(ssh_user, ssh_keypair).await?;
    if !auth_ok {
        bail!("SSH authentication rejected for user={ssh_user} key={}", ssh_key_path.display());
    }
    eprintln!("[ssh] authenticated successfully");
    Ok(ssh_handle)
}

pub(crate) async fn open_terminal_channel(
    ssh_handle: &mut Handle<SshClient>,
) -> Result<Channel<client::Msg>> {
    eprintln!("[ssh] opening terminal channel");
    let ssh_channel = ssh_handle.channel_open_session().await?;
    ssh_channel.request_pty(false, "xterm-256color", 80, 24, 0, 0, &[]).await?;
    ssh_channel.exec(false, "bash -c 'claude; exec bash'").await?;
    eprintln!("[ssh] terminal channel opened");
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
