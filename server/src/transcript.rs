use anyhow::Result;
use serde::Serialize;
use std::path::PathBuf;
use tokio::io::AsyncReadExt;

use crate::ssh::{connect_ssh, open_sftp_session};

#[derive(Serialize)]
pub(crate) struct TranscriptResponse {
    pub title: Option<String>,
    pub messages: Vec<TranscriptMessage>,
}

#[derive(Serialize)]
pub(crate) struct TranscriptMessage {
    pub role: String,
    pub content: Vec<serde_json::Value>,
}

pub(crate) async fn fetch_transcript(
    guest_ip: &str,
    ssh_key_path: &PathBuf,
    ssh_user: &str,
    vm_host_key_path: &PathBuf,
    session_id: &str,
) -> Result<TranscriptResponse> {
    let mut ssh_handle = connect_ssh(guest_ip, ssh_key_path, ssh_user, vm_host_key_path).await?;
    let sftp = open_sftp_session(&mut ssh_handle).await?;
    let transcript_path = build_transcript_path(session_id);
    let mut file = sftp.open(&transcript_path).await?;
    let mut contents = String::new();
    file.read_to_string(&mut contents).await?;
    parse_transcript(&contents)
}

fn build_transcript_path(session_id: &str) -> String {
    format!("/home/ubuntu/.claude/projects/-home-ubuntu/{session_id}.jsonl")
}

fn parse_transcript(contents: &str) -> Result<TranscriptResponse> {
    let mut title = None;
    let mut messages = Vec::new();
    for line in contents.lines() {
        let Ok(entry) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let type_str = entry["type"].as_str().unwrap_or("");
        match type_str {
            "summary" => {
                title = entry["summary"].as_str().map(|s| s.to_owned());
            }
            "user" | "assistant" => {
                if let Some(transcript_message) = extract_transcript_message(&entry, type_str) {
                    messages.push(transcript_message);
                }
            }
            _ => {}
        }
    }
    Ok(TranscriptResponse { title, messages })
}

fn extract_transcript_message(
    entry: &serde_json::Value,
    type_str: &str,
) -> Option<TranscriptMessage> {
    let message = &entry["message"];
    let role = message["role"].as_str().unwrap_or(type_str).to_owned();
    let content = message["content"].as_array()?.clone();
    if type_str == "user"
        && content
            .iter()
            .all(|b| b["type"].as_str() == Some("tool_result"))
    {
        return None;
    }
    Some(TranscriptMessage { role, content })
}
