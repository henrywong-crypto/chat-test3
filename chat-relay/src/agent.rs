use anyhow::{Context, Result};
use bytes::Bytes;
use futures::stream::Stream;
use russh::{Channel, ChannelMsg, client};
use serde::Serialize;
use ssh_client::{SshClient, connect_ssh, open_direct_streamlocal_channel};
use std::{
    net::Ipv4Addr,
    path::{Path, PathBuf},
    str::from_utf8,
    sync::{Arc, Mutex},
};
use tokio::{
    sync::mpsc,
    time::{Duration, Instant, interval, sleep, timeout},
};
use tokio_stream::wrappers::ReceiverStream;
use tracing::{debug, error, info};

const HEARTBEAT_SECS: u64 = 60;
const SEND_TIMEOUT_SECS: u64 = 30;
const AGENT_SOCKET_WAIT_SECS: u64 = 60;

pub enum AgentMessage {
    Query {
        task_id: String,
        content: String,
        session_id: Option<String>,
        work_dir: Option<String>,
    },
    Hello {
        task_id: String,
    },
    QuestionAnswer {
        request_id: String,
        answers: serde_json::Value,
    },
    Interrupt {
        task_id: String,
    },
}

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
}

impl VmRelayHandle {
    pub fn is_alive(&self) -> bool {
        !self.inbound_tx.is_closed()
    }

    pub fn register_sse_subscriber(&self) -> impl Stream<Item = Bytes> + use<> {
        let (tx, rx) = mpsc::channel::<Bytes>(1);
        *self.sse_output.lock().unwrap() = Some(tx);
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
    let ssh_key_path = ssh_key_path.to_owned();
    let ssh_user = ssh_user.to_owned();
    let vm_host_key_path = vm_host_key_path.to_owned();
    let relay_sse_output = sse_output.clone();
    tokio::spawn(async move {
        run_agent_relay(guest_ip, ssh_key_path, ssh_user, vm_host_key_path, inbound_rx, relay_sse_output).await
    });
    VmRelayHandle { inbound_tx, sse_output }
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
    let relay_result = connect_agent_relay(
        guest_ip,
        ssh_key_path,
        ssh_user,
        vm_host_key_path,
        inbound,
        sse_output.clone(),
    )
    .await;
    heartbeat_task.abort();
    if let Err(e) = relay_result {
        let error_payload = serde_json::json!({ "message": e.to_string() });
        let error_event = format!(
            "event: error_event\ndata: {}\n\n",
            serde_json::to_string(&error_payload).unwrap_or_default()
        );
        if let Some(tx) = take_live_sse_sender(&sse_output) {
            let _ = timeout(
                Duration::from_secs(SEND_TIMEOUT_SECS),
                tx.send(Bytes::from(error_event)),
            )
            .await;
        }
    }
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

async fn connect_agent_relay(
    guest_ip: Ipv4Addr,
    ssh_key_path: PathBuf,
    ssh_user: String,
    vm_host_key_path: PathBuf,
    inbound: mpsc::Receiver<AgentMessage>,
    sse_output: Arc<Mutex<Option<mpsc::Sender<Bytes>>>>,
) -> Result<()> {
    let ssh_handle = connect_ssh(guest_ip, &ssh_key_path, &ssh_user, &vm_host_key_path).await?;
    info!("agent ssh connected");
    let ssh_channel = open_agent_channel(&ssh_handle).await?;
    run_relay(ssh_handle, ssh_channel, inbound, sse_output).await
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
                        let payload = build_query_payload(&task_id, &content, session_id.as_deref(), work_dir.as_deref())?;
                        let line = format!("{payload}\n");
                        ssh_channel.data(Bytes::from(line).as_ref()).await?;
                        info!("query sent to agent");
                    }
                    Some(AgentMessage::Hello { task_id }) => {
                        info!("sending hello to agent  task_id={task_id}");
                        let payload = build_hello_payload(&task_id)?;
                        let line = format!("{payload}\n");
                        ssh_channel.data(Bytes::from(line).as_ref()).await?;
                        info!("hello sent to agent");
                    }
                    Some(AgentMessage::QuestionAnswer { request_id, answers }) => {
                        info!("sending question answer to agent  request_id={request_id}");
                        let payload = build_question_answer_payload(&request_id, &answers)?;
                        let line = format!("{payload}\n");
                        ssh_channel.data(Bytes::from(line).as_ref()).await?;
                        info!("question answer sent to agent");
                    }
                    Some(AgentMessage::Interrupt { task_id }) => {
                        info!("sending interrupt to agent  task_id={task_id}");
                        let payload = build_interrupt_payload(&task_id)?;
                        let line = format!("{payload}\n");
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

#[derive(Serialize)]
struct InterruptPayload<'a> {
    #[serde(rename = "type")]
    type_: &'a str,
    task_id: &'a str,
}

fn build_interrupt_payload(task_id: &str) -> Result<String> {
    let interrupt_payload = InterruptPayload {
        type_: "interrupt",
        task_id,
    };
    Ok(serde_json::to_string(&interrupt_payload)?)
}

#[derive(Serialize)]
struct HelloPayload<'a> {
    #[serde(rename = "type")]
    type_: &'a str,
    task_id: &'a str,
}

fn build_hello_payload(task_id: &str) -> Result<String> {
    let hello_payload = HelloPayload {
        type_: "hello",
        task_id,
    };
    Ok(serde_json::to_string(&hello_payload)?)
}

#[derive(Serialize)]
struct QueryPayload<'a> {
    #[serde(rename = "type")]
    type_: &'a str,
    task_id: &'a str,
    content: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    session_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    work_dir: Option<&'a str>,
}

fn build_query_payload(task_id: &str, content: &str, session_id: Option<&str>, work_dir: Option<&str>) -> Result<String> {
    let query_payload = QueryPayload {
        type_: "query",
        task_id,
        content,
        session_id,
        work_dir,
    };
    Ok(serde_json::to_string(&query_payload)?)
}

#[derive(Serialize)]
struct QuestionAnswerPayload<'a> {
    #[serde(rename = "type")]
    type_: &'a str,
    request_id: &'a str,
    answers: &'a serde_json::Value,
}

fn build_question_answer_payload(request_id: &str, answers: &serde_json::Value) -> Result<String> {
    let question_answer_payload = QuestionAnswerPayload {
        type_: "answer_question",
        request_id,
        answers,
    };
    Ok(serde_json::to_string(&question_answer_payload)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(json: &str) -> serde_json::Value {
        serde_json::from_str(json).expect("invalid JSON")
    }

    #[test]
    fn test_interrupt_type_field() {
        let json = build_interrupt_payload("abc-123").unwrap();
        assert_eq!(parse(&json)["type"], "interrupt");
    }

    #[test]
    fn test_interrupt_task_id_field() {
        let json = build_interrupt_payload("abc-123").unwrap();
        assert_eq!(parse(&json)["task_id"], "abc-123");
    }

    #[test]
    fn test_query_type_field() {
        let json = build_query_payload("task-1", "hello", None, None).unwrap();
        assert_eq!(parse(&json)["type"], "query");
    }

    #[test]
    fn test_content_field_is_present() {
        let json = build_query_payload("task-1", "hello world", None, None).unwrap();
        assert_eq!(parse(&json)["content"], "hello world");
    }

    #[test]
    fn test_session_id_included_when_some() {
        let json = build_query_payload("task-1", "hello", Some("abc-123"), None).unwrap();
        assert_eq!(parse(&json)["session_id"], "abc-123");
    }

    #[test]
    fn test_session_id_omitted_when_none() {
        let json = build_query_payload("task-1", "hello", None, None).unwrap();
        assert!(parse(&json).get("session_id").is_none());
    }

    #[test]
    fn test_special_characters_in_content_are_escaped() {
        let json = build_query_payload("task-1", "say \"hello\"\nand\\goodbye", None, None).unwrap();
        assert_eq!(parse(&json)["content"], "say \"hello\"\nand\\goodbye");
    }

    #[test]
    fn test_empty_content() {
        let json = build_query_payload("task-1", "", None, None).unwrap();
        assert_eq!(parse(&json)["content"], "");
    }

    #[test]
    fn test_task_id_included_in_query() {
        let json = build_query_payload("my-task-id", "hello", None, None).unwrap();
        assert_eq!(parse(&json)["task_id"], "my-task-id");
    }

    #[test]
    fn test_work_dir_included_when_some() {
        let json = build_query_payload("task-1", "hello", None, Some("/home/ubuntu")).unwrap();
        assert_eq!(parse(&json)["work_dir"], "/home/ubuntu");
    }

    #[test]
    fn test_work_dir_omitted_when_none() {
        let json = build_query_payload("task-1", "hello", None, None).unwrap();
        assert!(parse(&json).get("work_dir").is_none());
    }

    #[test]
    fn test_hello_type_field() {
        let json = build_hello_payload("task-abc").unwrap();
        assert_eq!(parse(&json)["type"], "hello");
    }

    #[test]
    fn test_hello_task_id_field() {
        let json = build_hello_payload("task-abc").unwrap();
        assert_eq!(parse(&json)["task_id"], "task-abc");
    }
}
