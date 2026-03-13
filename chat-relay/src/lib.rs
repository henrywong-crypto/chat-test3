use anyhow::Result;
use bytes::Bytes;
use futures::stream::Stream;
use russh::{client, Channel, ChannelMsg};
use serde::Serialize;
use ssh_client::{connect_ssh, open_exec_channel, SshClient};
use std::path::PathBuf;
use tokio::sync::mpsc;
use tokio::time::{interval, timeout, Duration};
use tokio_stream::wrappers::ReceiverStream;
use tracing::{debug, error, info};

const AGENT_CMD: &str = "bash -lc '\
    /usr/sbin/logrotate --force --state \"$HOME/.logrotate.status\" /etc/logrotate.d/agent; \
    PYTHONUNBUFFERED=1 /usr/local/bin/uv run /opt/agent.py 2> >(tee -a \"$HOME/agent.log\" >&2)\
'";
const SETTINGS_CMD: &str = "bash -lc '/usr/local/bin/uv run /opt/settings.py'";

const HEARTBEAT_SECS: u64 = 60;
const SEND_TIMEOUT_SECS: u64 = 30;

pub struct VmSettings {
    pub has_api_key: bool,
}

pub enum AgentMessage {
    Query {
        content: String,
        session_id: Option<String>,
    },
    Abort,
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
    guest_ip: &str,
    ssh_key_path: &PathBuf,
    ssh_user: &str,
    vm_host_key_path: &PathBuf,
) -> Result<VmSettings> {
    let mut ssh_handle = connect_ssh(guest_ip, ssh_key_path, ssh_user, vm_host_key_path).await?;
    let command = "{\"type\":\"get\"}\n";
    let mut channel = open_exec_channel(&mut ssh_handle, SETTINGS_CMD).await?;
    channel
        .data(Bytes::from(command.as_bytes()).as_ref())
        .await?;
    let mut stdout = String::new();
    loop {
        match channel.wait().await {
            Some(ChannelMsg::Data { ref data }) => {
                stdout.push_str(std::str::from_utf8(data).unwrap_or(""));
            }
            Some(ChannelMsg::ExitStatus { .. }) | None => break,
            _ => {}
        }
    }
    let response: serde_json::Value = serde_json::from_str(&stdout).unwrap_or_default();
    Ok(VmSettings {
        has_api_key: response
            .get("has_api_key")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
    })
}

pub async fn set_vm_settings(
    guest_ip: &str,
    ssh_key_path: &PathBuf,
    ssh_user: &str,
    vm_host_key_path: &PathBuf,
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
    channel.data(Bytes::from(cmd_line).as_ref()).await?;
    loop {
        match channel.wait().await {
            Some(ChannelMsg::ExitStatus { .. }) | None => break,
            _ => {}
        }
    }
    Ok(())
}

pub fn start_agent_relay(
    guest_ip: String,
    ssh_key_path: PathBuf,
    ssh_user: String,
    vm_host_key_path: PathBuf,
    inbound: mpsc::Receiver<AgentMessage>,
) -> impl Stream<Item = Bytes> {
    let (tx, rx) = mpsc::channel::<Bytes>(1);
    tokio::spawn(run_agent_relay(
        guest_ip,
        ssh_key_path,
        ssh_user,
        vm_host_key_path,
        inbound,
        tx,
    ));
    ReceiverStream::new(rx)
}

async fn run_agent_relay(
    guest_ip: String,
    ssh_key_path: PathBuf,
    ssh_user: String,
    vm_host_key_path: PathBuf,
    inbound: mpsc::Receiver<AgentMessage>,
    tx: mpsc::Sender<Bytes>,
) {
    // Keep the SSE connection alive while SSH is being established (can take up to 60s
    // while waiting for the VM to become reachable). Without this, browsers close idle
    // connections before the relay is ready.
    let heartbeat_tx = tx.clone();
    let heartbeat_task = tokio::spawn(async move {
        let mut interval = interval(Duration::from_secs(HEARTBEAT_SECS));
        // Do NOT skip the first tick: fire immediately to flush nginx proxy buffers
        // so the browser receives the HTTP 200 headers and fires onopen without delay.
        loop {
            interval.tick().await;
            if heartbeat_tx
                .send(Bytes::from_static(b": keep-alive\n\n"))
                .await
                .is_err()
            {
                break;
            }
        }
    });
    let relay_result = connect_agent_relay(
        &guest_ip,
        &ssh_key_path,
        &ssh_user,
        &vm_host_key_path,
        inbound,
        tx.clone(),
    )
    .await;
    heartbeat_task.abort();
    if let Err(e) = relay_result {
        let error_payload = serde_json::json!({ "message": e.to_string() });
        let error_event = format!(
            "event: error_event\ndata: {}\n\n",
            serde_json::to_string(&error_payload).unwrap_or_default()
        );
        let _ = timeout(
            Duration::from_secs(SEND_TIMEOUT_SECS),
            tx.send(Bytes::from(error_event)),
        )
        .await;
    }
}

async fn connect_agent_relay(
    guest_ip: &str,
    ssh_key_path: &PathBuf,
    ssh_user: &str,
    vm_host_key_path: &PathBuf,
    inbound: mpsc::Receiver<AgentMessage>,
    tx: mpsc::Sender<Bytes>,
) -> Result<()> {
    let mut ssh_handle = connect_ssh(guest_ip, ssh_key_path, ssh_user, vm_host_key_path).await?;
    info!("agent ssh channel opened");
    let ssh_channel = open_exec_channel(&mut ssh_handle, AGENT_CMD).await?;
    run_relay(ssh_handle, ssh_channel, inbound, tx).await
}

async fn run_relay(
    _ssh_handle: client::Handle<SshClient>, // keeps the SSH connection alive for the duration of the relay
    mut ssh_channel: Channel<client::Msg>,
    mut inbound: mpsc::Receiver<AgentMessage>,
    tx: mpsc::Sender<Bytes>,
) -> Result<()> {
    let mut heartbeat = interval(Duration::from_secs(HEARTBEAT_SECS));
    heartbeat.tick().await; // consume the immediate first tick
    loop {
        tokio::select! {
            biased;
            msg = inbound.recv() => {
                match msg {
                    None => {
                        info!("inbound channel closed, ending relay");
                        break;
                    }
                    Some(AgentMessage::Abort) => {
                        info!("abort received, ending relay");
                        break;
                    }
                    Some(AgentMessage::Query { content, session_id }) => {
                        info!("sending query to agent  content_len={}", content.len());
                        let payload = build_query_payload(&content, session_id.as_deref())?;
                        let line = format!("{payload}\n");
                        ssh_channel.data(Bytes::from(line).as_ref()).await?;
                        info!("query sent to agent");
                    }
                }
            }
            msg = ssh_channel.wait() => {
                match msg {
                    Some(ChannelMsg::Data { ref data }) => {
                        info!("received stdout from agent");
                        match timeout(Duration::from_secs(SEND_TIMEOUT_SECS), tx.send(Bytes::copy_from_slice(data))).await {
                            Ok(Ok(())) => {}
                            Ok(Err(_)) => {
                                info!("sse receiver dropped, ending relay");
                                break;
                            }
                            Err(_) => {
                                error!("send timed out, sse consumer likely stuck");
                                break;
                            }
                        }
                    }
                    Some(ChannelMsg::ExtendedData { ref data, .. }) => {
                        if let Ok(text) = std::str::from_utf8(data) {
                            for stderr_line in text.lines() {
                                if !stderr_line.is_empty() {
                                    debug!("{stderr_line}");
                                }
                            }
                        }
                    }
                    Some(ChannelMsg::ExitStatus { exit_status }) => {
                        info!("agent exited  status={exit_status}");
                        break;
                    }
                    None => {
                        info!("ssh channel closed");
                        break;
                    }
                    Some(other) => {
                        info!("unexpected ssh channel message  msg={other:?}");
                    }
                }
            }
            _ = heartbeat.tick() => {
                match timeout(Duration::from_secs(SEND_TIMEOUT_SECS), tx.send(Bytes::from_static(b": keep-alive\n\n"))).await {
                    Ok(Ok(())) => {}
                    Ok(Err(_)) => {
                        info!("sse receiver dropped during heartbeat, ending relay");
                        break;
                    }
                    Err(_) => {
                        error!("heartbeat send timed out, sse consumer likely stuck");
                        break;
                    }
                }
            }
        }
    }
    info!("agent relay ended");
    Ok(())
}

#[derive(Serialize)]
struct QueryPayload<'a> {
    #[serde(rename = "type")]
    kind: &'a str,
    content: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    session_id: Option<&'a str>,
}

fn build_query_payload(content: &str, session_id: Option<&str>) -> Result<String> {
    let query_payload = QueryPayload {
        kind: "query",
        content,
        session_id,
    };
    Ok(serde_json::to_string(&query_payload)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(json: &str) -> serde_json::Value {
        serde_json::from_str(json).expect("invalid JSON")
    }

    #[test]
    fn test_type_field_is_always_query() {
        let json = build_query_payload("hello", None).unwrap();
        assert_eq!(parse(&json)["type"], "query");
    }

    #[test]
    fn test_content_field_is_present() {
        let json = build_query_payload("hello world", None).unwrap();
        assert_eq!(parse(&json)["content"], "hello world");
    }

    #[test]
    fn test_session_id_included_when_some() {
        let json = build_query_payload("hello", Some("abc-123")).unwrap();
        assert_eq!(parse(&json)["session_id"], "abc-123");
    }

    #[test]
    fn test_session_id_omitted_when_none() {
        let json = build_query_payload("hello", None).unwrap();
        assert!(parse(&json).get("session_id").is_none());
    }

    #[test]
    fn test_special_characters_in_content_are_escaped() {
        let json = build_query_payload("say \"hello\"\nand\\goodbye", None).unwrap();
        assert_eq!(parse(&json)["content"], "say \"hello\"\nand\\goodbye");
    }

    #[test]
    fn test_empty_content() {
        let json = build_query_payload("", None).unwrap();
        assert_eq!(parse(&json)["content"], "");
    }
}
