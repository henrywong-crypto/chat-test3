use anyhow::Result;
use chrono::{DateTime, TimeZone, Utc};
use russh_sftp::client::fs::DirEntry;
use serde::Serialize;
use std::path::PathBuf;
use tokio::io::AsyncReadExt;

use crate::ssh::{connect_ssh, open_sftp_session};

#[derive(Serialize)]
pub(crate) struct SessionEntry {
    pub session_id: String,
    pub title: String,
    pub last_active_at: DateTime<Utc>,
}

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

pub(crate) async fn list_sessions(
    guest_ip: &str,
    ssh_key_path: &PathBuf,
    ssh_user: &str,
    vm_host_key_path: &PathBuf,
) -> Result<Vec<SessionEntry>> {
    let mut ssh_handle = connect_ssh(guest_ip, ssh_key_path, ssh_user, vm_host_key_path).await?;
    let sftp = open_sftp_session(&mut ssh_handle).await?;
    let dir_path = "/home/ubuntu/.claude/projects/-home-ubuntu";
    let read_dir = sftp.read_dir(dir_path).await?;
    let session_entries = collect_session_entries(read_dir.collect());
    Ok(session_entries)
}

fn collect_session_entries(dir_entries: Vec<DirEntry>) -> Vec<SessionEntry> {
    let mut session_entries: Vec<SessionEntry> = dir_entries
        .iter()
        .filter_map(build_session_entry)
        .collect();
    session_entries.sort_by(|a, b| b.last_active_at.cmp(&a.last_active_at));
    session_entries
}

fn build_session_entry(entry: &DirEntry) -> Option<SessionEntry> {
    let session_id = entry.file_name().strip_suffix(".jsonl")?.to_owned();
    let mtime = entry.metadata().mtime.unwrap_or(0);
    let last_active_at = Utc
        .timestamp_opt(mtime as i64, 0)
        .single()
        .unwrap_or_default();
    Some(SessionEntry { session_id: session_id.clone(), title: session_id, last_active_at })
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
                    if title.is_none() && type_str == "user" {
                        title = extract_title_from_message(&transcript_message);
                    }
                    messages.push(transcript_message);
                }
            }
            _ => {}
        }
    }
    Ok(TranscriptResponse { title, messages })
}

fn extract_title_from_message(message: &TranscriptMessage) -> Option<String> {
    let text = message.content.iter().find_map(|b| b["text"].as_str())?;
    let title: String = text.chars().take(60).collect();
    if title.is_empty() { None } else { Some(title) }
}

fn extract_transcript_message(
    entry: &serde_json::Value,
    type_str: &str,
) -> Option<TranscriptMessage> {
    let message = &entry["message"];
    let role = message["role"].as_str().unwrap_or(type_str).to_owned();
    let content = normalize_content(&message["content"]);
    if content.is_empty() {
        return None;
    }
    if type_str == "user" && content.iter().all(|b| b["type"].as_str() == Some("tool_result")) {
        return None;
    }
    Some(TranscriptMessage { role, content })
}

fn normalize_content(raw: &serde_json::Value) -> Vec<serde_json::Value> {
    match raw {
        serde_json::Value::String(text) => {
            vec![serde_json::json!({"type": "text", "text": text})]
        }
        serde_json::Value::Array(blocks) => blocks
            .iter()
            .filter(|b| b["type"].as_str() != Some("thinking"))
            .cloned()
            .collect(),
        _ => vec![],
    }
}
