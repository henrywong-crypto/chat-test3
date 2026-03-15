use anyhow::{Context, Result, anyhow};
use bytes::Bytes;
use russh::{ChannelMsg, client};
use ssh_client::{SshClient, connect_ssh, open_exec_channel};
use std::{net::Ipv4Addr, path::Path, str::from_utf8, time::Duration};
use tokio::time::timeout;

const SETTINGS_CMD: &str = "bash -lc '/usr/local/bin/uv run /opt/settings.py'";
const CHANNEL_SEND_TIMEOUT_SECS: u64 = 30;
const CHANNEL_WAIT_TIMEOUT_SECS: u64 = 30;

pub struct VmSettings {
    pub has_api_key: bool,
}

pub fn build_api_key_settings_json(
    api_key: &str,
    base_url: Option<&str>,
    haiku_model: &str,
    sonnet_model: &str,
    opus_model: &str,
) -> String {
    let mut env = serde_json::json!({
        "ANTHROPIC_AUTH_TOKEN": api_key,
        "ANTHROPIC_DEFAULT_HAIKU_MODEL": haiku_model,
        "ANTHROPIC_DEFAULT_SONNET_MODEL": sonnet_model,
        "ANTHROPIC_DEFAULT_OPUS_MODEL": opus_model,
        "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC": "1",
    });
    if let Some(url) = base_url {
        env["ANTHROPIC_BASE_URL"] = serde_json::Value::String(url.to_string());
    }
    serde_json::json!({
        "$schema": "https://json.schemastore.org/claude-code-settings.json",
        "env": env,
        "skipWebFetchPreflight": true,
    })
    .to_string()
}

pub async fn get_vm_settings(
    guest_ip: Ipv4Addr,
    ssh_key_path: &Path,
    ssh_user: &str,
    vm_host_key_path: &Path,
) -> Result<VmSettings> {
    let mut ssh_handle = connect_ssh(guest_ip, ssh_key_path, ssh_user, vm_host_key_path).await?;
    let command = "{\"type\":\"get\"}\n";
    let mut channel = open_exec_channel(&mut ssh_handle, SETTINGS_CMD).await?;
    timeout(
        Duration::from_secs(CHANNEL_SEND_TIMEOUT_SECS),
        channel.data(Bytes::from(command.as_bytes()).as_ref()),
    )
    .await
    .context("SSH channel send timed out")?
    .context("SSH channel send failed")?;
    let mut stdout = String::new();
    loop {
        match timeout(Duration::from_secs(CHANNEL_WAIT_TIMEOUT_SECS), channel.wait()).await {
            Ok(Some(ChannelMsg::Data { ref data })) => {
                stdout.push_str(from_utf8(data).unwrap_or(""));
            }
            Ok(Some(ChannelMsg::ExitStatus { .. })) | Ok(None) => break,
            Ok(_) => {}
            Err(_) => return Err(anyhow!("SSH channel read timed out")),
        }
    }
    let response: serde_json::Value =
        serde_json::from_str(&stdout).context("failed to parse settings response")?;
    Ok(VmSettings {
        has_api_key: response
            .get("has_api_key")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
    })
}

pub async fn set_vm_settings(
    guest_ip: Ipv4Addr,
    ssh_key_path: &Path,
    ssh_user: &str,
    vm_host_key_path: &Path,
    content: &str,
) -> Result<()> {
    let mut ssh_handle = connect_ssh(guest_ip, ssh_key_path, ssh_user, vm_host_key_path).await?;
    send_settings_command(&mut ssh_handle, content).await
}

async fn send_settings_command(
    ssh_handle: &mut client::Handle<SshClient>,
    content: &str,
) -> Result<()> {
    let command = serde_json::to_string(&serde_json::json!({
        "type": "set",
        "content": content,
    }))?;
    let cmd_line = format!("{command}\n");
    let mut channel = open_exec_channel(ssh_handle, SETTINGS_CMD).await?;
    timeout(
        Duration::from_secs(CHANNEL_SEND_TIMEOUT_SECS),
        channel.data(Bytes::from(cmd_line).as_ref()),
    )
    .await
    .context("SSH channel send timed out")?
    .context("SSH channel send failed")?;
    loop {
        match timeout(Duration::from_secs(CHANNEL_WAIT_TIMEOUT_SECS), channel.wait()).await {
            Ok(Some(ChannelMsg::ExitStatus { .. })) | Ok(None) => break,
            Ok(_) => {}
            Err(_) => return Err(anyhow!("SSH channel read timed out")),
        }
    }
    Ok(())
}
