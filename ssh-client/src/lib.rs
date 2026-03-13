use anyhow::{bail, Context, Result};
use russh::{
    client::{self, Handle},
    keys::{load_public_key, load_secret_key, PrivateKey, PrivateKeyWithHashAlg, PublicKey},
    Channel,
};
use std::{path::PathBuf, sync::Arc, time::Duration};

pub struct SshClient {
    vm_host_key: Option<PublicKey>,
}

impl client::Handler for SshClient {
    type Error = russh::Error;

    fn check_server_key(
        &mut self,
        server_public_key: &PublicKey,
    ) -> impl std::future::Future<Output = Result<bool, Self::Error>> + Send {
        let matches = self
            .vm_host_key
            .as_ref()
            .map_or(true, |key| server_public_key == key);
        async move { Ok(matches) }
    }
}

pub async fn connect_ssh(
    guest_ip: &str,
    ssh_key_path: &PathBuf,
    ssh_user: &str,
    vm_host_key_path: &PathBuf,
) -> Result<Handle<SshClient>> {
    let vm_host_key = load_vm_host_key(vm_host_key_path)?;
    let ssh_keypair = load_ssh_keypair(ssh_key_path)?;
    let ssh_config = Arc::new(client::Config::default());
    let addr = format!("{guest_ip}:22");
    let connect_deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    let mut ssh_handle = loop {
        let ssh_client = SshClient {
            vm_host_key: vm_host_key.clone(),
        };
        match client::connect(ssh_config.clone(), addr.as_str(), ssh_client).await {
            Ok(ssh_handle) => break ssh_handle,
            Err(_) if tokio::time::Instant::now() < connect_deadline => {
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            Err(connect_error) => bail!("SSH connect timed out: {connect_error}"),
        }
    };
    authenticate_ssh_handle(&mut ssh_handle, ssh_user, ssh_keypair).await?;
    Ok(ssh_handle)
}

fn load_vm_host_key(vm_host_key_path: &PathBuf) -> Result<Option<PublicKey>> {
    if vm_host_key_path.as_os_str().is_empty() {
        return Ok(None);
    }
    let vm_host_key = load_public_key(vm_host_key_path).context("failed to load VM host key")?;
    Ok(Some(vm_host_key))
}

fn load_ssh_keypair(ssh_key_path: &PathBuf) -> Result<Arc<PrivateKey>> {
    let ssh_keypair =
        Arc::new(load_secret_key(ssh_key_path, None).context("failed to load SSH key")?);
    Ok(ssh_keypair)
}

async fn authenticate_ssh_handle(
    ssh_handle: &mut Handle<SshClient>,
    ssh_user: &str,
    ssh_keypair: Arc<PrivateKey>,
) -> Result<()> {
    let auth_result = ssh_handle
        .authenticate_publickey(ssh_user, PrivateKeyWithHashAlg::new(ssh_keypair, None))
        .await?;
    if !auth_result.success() {
        bail!("SSH authentication rejected for user={ssh_user}");
    }
    Ok(())
}

pub async fn open_terminal_channel(
    ssh_handle: &mut Handle<SshClient>,
) -> Result<Channel<client::Msg>> {
    let ssh_channel = ssh_handle.channel_open_session().await?;
    ssh_channel
        .request_pty(false, "xterm-256color", 80, 24, 0, 0, &[])
        .await?;
    ssh_channel
        .exec(false, "bash -ic 'claude; exec bash'")
        .await?;
    Ok(ssh_channel)
}

pub async fn open_exec_channel(
    ssh_handle: &mut Handle<SshClient>,
    command: &str,
) -> Result<Channel<client::Msg>> {
    let ssh_channel = ssh_handle.channel_open_session().await?;
    ssh_channel.exec(false, command).await?;
    Ok(ssh_channel)
}
