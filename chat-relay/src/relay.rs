use std::path::PathBuf;

use anyhow::Result;
use bytes::Bytes;
use futures::stream::Stream;
use russh::{client, Channel, ChannelMsg};
use ssh_client::{connect_ssh, open_exec_channel, SshClient};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tracing::info;

const AGENT_CMD: &str = "bash -lc '\
    /usr/sbin/logrotate --state /dev/null <(printf \"%s/agent.log {\\n  rotate 2\\n  size 10M\\n  missingok\\n  nocreate\\n}\\n\" \"$HOME\"); \
    PYTHONUNBUFFERED=1 /usr/local/bin/uv run /opt/agent.py 2> >(tee -a \"$HOME/agent.log\" >&2)\
'";

pub enum AgentMessage {
    Query { content: String, session_id: Option<String> },
    Abort,
}

pub async fn start_agent_relay(
    guest_ip: &str,
    ssh_key_path: &PathBuf,
    ssh_user: &str,
    vm_host_key_path: &PathBuf,
    inbound: mpsc::Receiver<AgentMessage>,
    vm_id: String,
) -> Result<impl Stream<Item = Bytes>> {
    let mut ssh_handle = connect_ssh(guest_ip, ssh_key_path, ssh_user, vm_host_key_path).await?;
    info!("agent ssh channel opened");
    let ssh_channel = open_exec_channel(&mut ssh_handle, AGENT_CMD).await?;
    let (tx, rx) = mpsc::channel::<Bytes>(1);
    tokio::spawn(run_relay(ssh_handle, ssh_channel, inbound, tx, vm_id));
    Ok(ReceiverStream::new(rx))
}

async fn run_relay(
    _ssh_handle: client::Handle<SshClient>, // keeps the SSH connection alive for the duration of the relay
    mut ssh_channel: Channel<client::Msg>,
    mut inbound: mpsc::Receiver<AgentMessage>,
    tx: mpsc::Sender<Bytes>,
    vm_id: String,
) -> Result<()> {
    loop {
        tokio::select! {
            biased;
            msg = inbound.recv() => {
                match msg {
                    None | Some(AgentMessage::Abort) => break,
                    Some(AgentMessage::Query { content, session_id }) => {
                        let payload = build_query_payload(&content, session_id.as_deref())?;
                        let line = format!("{payload}\n");
                        ssh_channel.data(Bytes::from(line).as_ref()).await?;
                    }
                }
            }
            msg = ssh_channel.wait() => {
                match msg {
                    Some(ChannelMsg::Data { ref data }) => {
                        if tx.send(Bytes::copy_from_slice(data)).await.is_err() {
                            break;
                        }
                    }
                    Some(ChannelMsg::ExtendedData { ref data, .. }) => {
                        if let Ok(text) = std::str::from_utf8(data) {
                            for stderr_line in text.lines() {
                                if !stderr_line.is_empty() {
                                    info!(vm_id = %vm_id, "{stderr_line}");
                                }
                            }
                        }
                    }
                    Some(ChannelMsg::ExitStatus { exit_status }) => {
                        info!(vm_id = %vm_id, "agent exited  status={exit_status}");
                        break;
                    }
                    None => break,
                    _ => {}
                }
            }
        }
    }
    info!(vm_id = %vm_id, "agent relay ended");
    Ok(())
}

fn build_query_payload(content: &str, session_id: Option<&str>) -> Result<String> {
    let v = match session_id {
        Some(id) => serde_json::json!({"type": "query", "content": content, "session_id": id}),
        None => serde_json::json!({"type": "query", "content": content}),
    };
    Ok(serde_json::to_string(&v)?)
}
