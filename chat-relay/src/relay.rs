use anyhow::Result;
use axum::response::sse::Event;
use bytes::Bytes;
use futures::stream::Stream;
use russh::{client, Channel, ChannelMsg};
use std::path::PathBuf;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tracing::info;

use ssh_client::{connect_ssh, open_exec_channel, SshClient};

const AGENT_CMD: &str = "bash -lc 'PYTHONUNBUFFERED=1 /usr/local/bin/uv run /opt/agent.py 2> >(tee -a \"$HOME/agent.log\" >&2)'";

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
) -> Result<impl Stream<Item = anyhow::Result<Event>>> {
    let mut ssh_handle = connect_ssh(guest_ip, ssh_key_path, ssh_user, vm_host_key_path).await?;
    info!("agent ssh channel opened");
    let ssh_channel = open_exec_channel(&mut ssh_handle, AGENT_CMD).await?;
    let (event_tx, event_rx) = mpsc::channel::<anyhow::Result<Event>>(1);
    tokio::spawn(run_relay(ssh_handle, ssh_channel, inbound, event_tx, vm_id));
    Ok(ReceiverStream::new(event_rx))
}

async fn run_relay(
    _ssh_handle: client::Handle<SshClient>, // keeps the SSH connection alive for the duration of the relay
    mut ssh_channel: Channel<client::Msg>,
    mut inbound: mpsc::Receiver<AgentMessage>,
    event_tx: mpsc::Sender<anyhow::Result<Event>>,
    vm_id: String,
) {
    let mut line_buf = String::new();
    let mut pending_event_name: Option<String> = None;
    loop {
        tokio::select! {
            biased;
            msg = inbound.recv() => {
                match msg {
                    None | Some(AgentMessage::Abort) => break,
                    Some(AgentMessage::Query { content, session_id }) => {
                        info!(vm_id = %format_short_id(&vm_id), "query");
                        let payload = build_query_payload(&content, session_id.as_deref());
                        let line = format!("{payload}\n");
                        if ssh_channel.data(Bytes::from(line).as_ref()).await.is_err() {
                            break;
                        }
                    }
                }
            }
            _ = std::future::ready(()), if line_buf.contains('\n') => {
                if let Some(pos) = line_buf.find('\n') {
                    let line = line_buf[..pos].trim_end_matches('\r').to_owned();
                    line_buf.drain(..=pos);
                    if !forward_sse_line(line, &mut pending_event_name, &event_tx, &vm_id).await {
                        break;
                    }
                }
            }
            msg = ssh_channel.wait() => {
                match msg {
                    Some(ChannelMsg::Data { ref data }) => {
                        line_buf.push_str(std::str::from_utf8(data).unwrap_or(""));
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
                        info!(vm_id = %format_short_id(&vm_id), "agent exited  status={exit_status}");
                        break;
                    }
                    None => break,
                    _ => {}
                }
            }
        }
    }
    info!(vm_id = %format_short_id(&vm_id), "agent relay ended");
}

/// Parses one SSE line and forwards a data event if applicable.
/// Returns false when the SSE client has disconnected (event_tx closed).
async fn forward_sse_line(
    line: String,
    pending_event_name: &mut Option<String>,
    event_tx: &mpsc::Sender<anyhow::Result<Event>>,
    vm_id: &str,
) -> bool {
    if let Some(name) = line.strip_prefix("event: ") {
        *pending_event_name = Some(name.to_owned());
        true
    } else if let Some(payload) = line.strip_prefix("data: ") {
        let name = pending_event_name.take().unwrap_or_else(|| "message".to_owned());
        log_agent_event(vm_id, &name);
        let event = Event::default().event(name).data(payload.to_owned());
        event_tx.send(Ok(event)).await.is_ok()
    } else {
        true // blank line or : comment
    }
}

fn build_query_payload(content: &str, session_id: Option<&str>) -> String {
    let v = match session_id {
        Some(id) => serde_json::json!({"type": "query", "content": content, "session_id": id}),
        None => serde_json::json!({"type": "query", "content": content}),
    };
    serde_json::to_string(&v).unwrap_or_default()
}

fn log_agent_event(vm_id: &str, event_name: &str) {
    match event_name {
        "text_delta" | "thinking_delta" => {}
        other => info!(vm_id = %format_short_id(vm_id), "agent  {other}"),
    }
}

fn format_short_id(id: &str) -> &str {
    match id.char_indices().nth(8) {
        Some((i, _)) => &id[..i],
        None => id,
    }
}
