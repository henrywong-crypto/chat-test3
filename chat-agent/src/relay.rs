use anyhow::{Context, Result};
use bytes::Bytes;
use futures::stream::Stream;
use russh::{Channel, ChannelMsg, client};
use ssh_client::{SshClient, connect_ssh, open_direct_streamlocal_channel};
use std::{
    net::Ipv4Addr,
    path::{Path, PathBuf},
    str::from_utf8,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::{
    sync::mpsc,
    time::{Duration, Instant, interval, sleep, timeout},
};
use tokio_stream::wrappers::ReceiverStream;
use tracing::{debug, error, info};

use crate::{AgentMessage, HelloPayload, InterruptPayload, QueryPayload, QuestionAnswerPayload};

const HEARTBEAT_SECS: u64 = 60;
const SEND_TIMEOUT_SECS: u64 = 30;
const AGENT_SOCKET_WAIT_SECS: u64 = 60;
const RELAY_READY_EVENT: &[u8] = b"event: relay_ready\ndata: {}\n\n";

// One relay per VM: a single persistent SSH connection runs connector.py and forwards
// messages between the inbound channel (fed by POST handlers) and the SSE output channel
// (consumed by the browser). The relay task outlives individual SSE connections — when
// the browser disconnects and reconnects, register_sse_subscriber swaps in a fresh output
// channel without restarting connector.py. The relay task exits only when the inbound
// channel is dropped (VM removed) or the SSH connection closes.
#[derive(Clone)]
pub struct VmRelayHandle {
    inbound_tx: mpsc::Sender<AgentMessage>,
    sse_output: Arc<Mutex<Option<mpsc::Sender<Bytes>>>>,
    relay_connected: Arc<AtomicBool>,
}

impl VmRelayHandle {
    pub fn is_alive(&self) -> bool {
        !self.inbound_tx.is_closed()
    }

    pub fn register_sse_subscriber(&self) -> impl Stream<Item = Bytes> + use<> {
        // Use capacity 2: one slot for relay_ready (if already connected) and one for the
        // first real event, preventing a brief block at connection time.
        let (tx, rx) = mpsc::channel::<Bytes>(2);
        {
            let mut output = self.sse_output.lock().unwrap();
            if self.relay_connected.load(Ordering::Acquire) {
                let _ = tx.try_send(Bytes::from_static(RELAY_READY_EVENT));
            }
            *output = Some(tx);
        }
        ReceiverStream::new(rx)
    }

    pub fn inbound_tx(&self) -> &mpsc::Sender<AgentMessage> {
        &self.inbound_tx
    }
}

pub fn start_vm_relay(
    guest_ip: Ipv4Addr,
    ssh_key_path: &Path,
    ssh_user: &str,
    vm_host_key_path: &Path,
) -> VmRelayHandle {
    let (inbound_tx, inbound_rx) = mpsc::channel::<AgentMessage>(4);
    let sse_output: Arc<Mutex<Option<mpsc::Sender<Bytes>>>> = Arc::new(Mutex::new(None));
    let relay_connected: Arc<AtomicBool> = Arc::new(AtomicBool::new(false));
    let ssh_key_path = ssh_key_path.to_owned();
    let ssh_user = ssh_user.to_owned();
    let vm_host_key_path = vm_host_key_path.to_owned();
    let relay_sse_output = sse_output.clone();
    let relay_relay_connected = relay_connected.clone();
    tokio::spawn(async move {
        run_agent_relay(guest_ip, ssh_key_path, ssh_user, vm_host_key_path, inbound_rx, relay_sse_output, relay_relay_connected).await
    });
    VmRelayHandle { inbound_tx, sse_output, relay_connected }
}

fn take_live_sse_sender(sse_output: &Mutex<Option<mpsc::Sender<Bytes>>>) -> Option<mpsc::Sender<Bytes>> {
    let mut guard = sse_output.lock().unwrap();
    let sender = guard.as_ref()?;
    if sender.is_closed() {
        *guard = None;
        return None;
    }
    Some(sender.clone())
}

async fn run_agent_relay(
    guest_ip: Ipv4Addr,
    ssh_key_path: PathBuf,
    ssh_user: String,
    vm_host_key_path: PathBuf,
    inbound: mpsc::Receiver<AgentMessage>,
    sse_output: Arc<Mutex<Option<mpsc::Sender<Bytes>>>>,
    relay_connected: Arc<AtomicBool>,
) {
    let heartbeat_sse_output = sse_output.clone();
    let heartbeat_task = tokio::spawn(async move {
        let mut heartbeat_interval = interval(Duration::from_secs(HEARTBEAT_SECS));
        heartbeat_interval.tick().await;
        loop {
            heartbeat_interval.tick().await;
            let Some(tx) = take_live_sse_sender(&heartbeat_sse_output) else {
                continue;
            };
            match timeout(
                Duration::from_secs(SEND_TIMEOUT_SECS),
                tx.send(Bytes::from_static(b": keep-alive\n\n")),
            )
            .await
            {
                Ok(Ok(())) => {}
                Ok(Err(_)) => {
                    info!("sse receiver dropped during startup heartbeat");
                }
                Err(_) => {
                    error!("startup heartbeat send timed out, sse consumer likely stuck");
                }
            }
        }
    });
    let connect_result = connect_ssh_and_open_channel(
        guest_ip,
        &ssh_key_path,
        &ssh_user,
        &vm_host_key_path,
    )
    .await;
    heartbeat_task.abort();
    match connect_result {
        Err(e) => {
            send_sse_error(&sse_output, e).await;
        }
        Ok((ssh_handle, ssh_channel)) => {
            relay_connected.store(true, Ordering::Release);
            if let Some(tx) = take_live_sse_sender(&sse_output) {
                match timeout(
                    Duration::from_secs(SEND_TIMEOUT_SECS),
                    tx.send(Bytes::from_static(RELAY_READY_EVENT)),
                )
                .await
                {
                    Ok(Ok(())) => {}
                    Ok(Err(_)) => {
                        info!("sse receiver dropped before relay_ready");
                    }
                    Err(_) => {
                        error!("relay_ready send timed out");
                    }
                }
            }
            if let Err(e) = run_relay(ssh_handle, ssh_channel, inbound, sse_output.clone()).await {
                send_sse_error(&sse_output, e).await;
            }
        }
    }
}

async fn send_sse_error(sse_output: &Mutex<Option<mpsc::Sender<Bytes>>>, e: anyhow::Error) {
    let error_payload = serde_json::json!({ "message": e.to_string() });
    let error_event = format!(
        "event: error_event\ndata: {}\n\n",
        serde_json::to_string(&error_payload).unwrap_or_default()
    );
    if let Some(tx) = take_live_sse_sender(sse_output) {
        let _ = timeout(
            Duration::from_secs(SEND_TIMEOUT_SECS),
            tx.send(Bytes::from(error_event)),
        )
        .await;
    }
}

async fn connect_ssh_and_open_channel(
    guest_ip: Ipv4Addr,
    ssh_key_path: &Path,
    ssh_user: &str,
    vm_host_key_path: &Path,
) -> Result<(client::Handle<SshClient>, Channel<client::Msg>)> {
    let ssh_handle = connect_ssh(guest_ip, ssh_key_path, ssh_user, vm_host_key_path).await?;
    info!("agent ssh connected");
    let ssh_channel = open_agent_channel(&ssh_handle).await?;
    Ok((ssh_handle, ssh_channel))
}

async fn open_agent_channel(ssh_handle: &client::Handle<SshClient>) -> Result<Channel<client::Msg>> {
    let deadline = Instant::now() + Duration::from_secs(AGENT_SOCKET_WAIT_SECS);
    loop {
        match open_direct_streamlocal_channel(ssh_handle, "/tmp/agent.sock").await {
            Ok(channel) => {
                info!("agent socket channel opened");
                return Ok(channel);
            }
            Err(e) if Instant::now() < deadline => {
                debug!("agent socket not ready, retrying: {e}");
                sleep(Duration::from_millis(500)).await;
            }
            Err(e) => return Err(e).context("timed out waiting for agent socket"),
        }
    }
}

async fn run_relay(
    _ssh_handle: client::Handle<SshClient>, // keeps the SSH connection alive for the duration of the relay
    mut ssh_channel: Channel<client::Msg>,
    mut inbound: mpsc::Receiver<AgentMessage>,
    sse_output: Arc<Mutex<Option<mpsc::Sender<Bytes>>>>,
) -> Result<()> {
    let mut heartbeat = interval(Duration::from_secs(HEARTBEAT_SECS));
    heartbeat.tick().await;
    loop {
        tokio::select! {
            biased;
            msg = inbound.recv() => {
                match msg {
                    None => {
                        info!("inbound channel closed, ending relay");
                        break;
                    }
                    Some(AgentMessage::Query { task_id, content, session_id, work_dir }) => {
                        info!("sending query to agent  content_len={}", content.len());
                        let payload = QueryPayload { type_: "query".to_string(), task_id, content, session_id, work_dir };
                        let line = format!("{}\n", serde_json::to_string(&payload)?);
                        ssh_channel.data(Bytes::from(line).as_ref()).await?;
                        info!("query sent to agent");
                    }
                    Some(AgentMessage::Hello { task_id }) => {
                        info!("sending hello to agent  task_id={task_id}");
                        let payload = HelloPayload { type_: "hello".to_string(), task_id };
                        let line = format!("{}\n", serde_json::to_string(&payload)?);
                        ssh_channel.data(Bytes::from(line).as_ref()).await?;
                        info!("hello sent to agent");
                    }
                    Some(AgentMessage::QuestionAnswer { request_id, answers }) => {
                        info!("sending question answer to agent  request_id={request_id}");
                        let payload = QuestionAnswerPayload { type_: "answer_question".to_string(), request_id, answers };
                        let line = format!("{}\n", serde_json::to_string(&payload)?);
                        ssh_channel.data(Bytes::from(line).as_ref()).await?;
                        info!("question answer sent to agent");
                    }
                    Some(AgentMessage::Interrupt { task_id }) => {
                        info!("sending interrupt to agent  task_id={task_id}");
                        let payload = InterruptPayload { type_: "interrupt".to_string(), task_id };
                        let line = format!("{}\n", serde_json::to_string(&payload)?);
                        ssh_channel.data(Bytes::from(line).as_ref()).await?;
                        info!("interrupt sent to agent");
                    }
                }
            }
            msg = ssh_channel.wait() => {
                match msg {
                    Some(ChannelMsg::Data { ref data }) => {
                        info!("received stdout from agent");
                        let Some(tx) = take_live_sse_sender(&sse_output) else {
                            continue;
                        };
                        match timeout(Duration::from_secs(SEND_TIMEOUT_SECS), tx.send(Bytes::copy_from_slice(data))).await {
                            Ok(Ok(())) => {}
                            Ok(Err(_)) => {
                                info!("sse receiver dropped");
                            }
                            Err(_) => {
                                error!("send timed out, sse consumer likely stuck");
                            }
                        }
                    }
                    Some(ChannelMsg::ExtendedData { ref data, .. }) => {
                        if let Ok(text) = from_utf8(data) {
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
                let Some(tx) = take_live_sse_sender(&sse_output) else {
                    continue;
                };
                match timeout(Duration::from_secs(SEND_TIMEOUT_SECS), tx.send(Bytes::from_static(b": keep-alive\n\n"))).await {
                    Ok(Ok(())) => {}
                    Ok(Err(_)) => {
                        info!("sse receiver dropped during heartbeat");
                    }
                    Err(_) => {
                        error!("heartbeat send timed out, sse consumer likely stuck");
                    }
                }
            }
        }
    }
    info!("agent relay ended");
    Ok(())
}
