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
use tracing::{error, info};

const AGENT_CMD: &str = "bash -lc '\
    /usr/sbin/logrotate --force /etc/logrotate.d/agent; \
    PYTHONUNBUFFERED=1 /usr/local/bin/uv run /opt/agent.py 2> >(tee -a \"$HOME/agent.log\" >&2)\
'";

const HEARTBEAT_SECS: u64 = 60;
const SEND_TIMEOUT_SECS: u64 = 30;

pub enum AgentMessage {
    Query {
        content: String,
        session_id: Option<String>,
    },
    Abort,
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
                        info!("received stdout from agent  bytes={}", data.len());
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
                                    info!("{stderr_line}");
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
