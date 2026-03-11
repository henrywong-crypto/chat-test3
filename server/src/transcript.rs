use anyhow::Result;
use chrono::{DateTime, TimeZone, Utc};
use russh_sftp::client::{fs::DirEntry, SftpSession};
use serde::Serialize;
use std::path::PathBuf;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};

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

const PROJECTS_DIR: &str = "/home/ubuntu/.claude/projects/-home-ubuntu";

pub(crate) async fn list_sessions(
    guest_ip: &str,
    ssh_key_path: &PathBuf,
    ssh_user: &str,
    vm_host_key_path: &PathBuf,
) -> Result<Vec<SessionEntry>> {
    let mut ssh_handle = connect_ssh(guest_ip, ssh_key_path, ssh_user, vm_host_key_path).await?;
    let sftp = open_sftp_session(&mut ssh_handle).await?;
    let dir_entries: Vec<DirEntry> = match sftp.read_dir(PROJECTS_DIR).await {
        Ok(rd) => rd.collect(),
        Err(_) => return Ok(vec![]),
    };
    build_session_entries(&sftp, dir_entries).await
}

async fn build_session_entries(
    sftp: &SftpSession,
    dir_entries: Vec<DirEntry>,
) -> Result<Vec<SessionEntry>> {
    let mut session_entries = Vec::new();
    for dir_entry in &dir_entries {
        if let Some(session_entry) = build_session_entry_with_title(sftp, dir_entry).await {
            session_entries.push(session_entry);
        }
    }
    session_entries.sort_by(|a, b| b.last_active_at.cmp(&a.last_active_at));
    Ok(session_entries)
}

async fn build_session_entry_with_title(
    sftp: &SftpSession,
    dir_entry: &DirEntry,
) -> Option<SessionEntry> {
    let session_id = dir_entry.file_name().strip_suffix(".jsonl")?.to_owned();
    if session_id.starts_with("agent-") {
        return None;
    }
    let mtime = dir_entry.metadata().mtime.unwrap_or(0);
    let last_active_at = Utc
        .timestamp_opt(mtime as i64, 0)
        .single()
        .unwrap_or_default();
    let path = format!("{PROJECTS_DIR}/{session_id}.jsonl");
    let title = fetch_session_title(sftp, &path)
        .await
        .unwrap_or_else(|| session_id.clone());
    Some(SessionEntry { session_id, title, last_active_at })
}

async fn fetch_session_title(sftp: &SftpSession, path: &str) -> Option<String> {
    let file = sftp.open(path).await.ok()?;
    let mut lines = BufReader::new(file).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let Ok(entry) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        match entry["type"].as_str().unwrap_or("") {
            "summary" => return entry["summary"].as_str().map(|s| s.to_owned()),
            "user" => {
                if let Some(msg) = extract_transcript_message(&entry, "user") {
                    if let Some(title) = extract_title_from_message(&msg) {
                        return Some(title);
                    }
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
fn extract_title_from_jsonl(chunk: &str) -> Option<String> {
    let mut first_user_title = None;
    for line in chunk.lines() {
        let Ok(entry) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        match entry["type"].as_str().unwrap_or("") {
            "summary" => return entry["summary"].as_str().map(|s| s.to_owned()),
            "user" => {
                if first_user_title.is_none() {
                    if let Some(msg) = extract_transcript_message(&entry, "user") {
                        first_user_title = extract_title_from_message(&msg);
                    }
                }
            }
            _ => {}
        }
    }
    first_user_title
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
    format!("{PROJECTS_DIR}/{session_id}.jsonl")
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

#[cfg(test)]
mod tests {
    use super::*;

    fn user_text(text: &str) -> String {
        serde_json::json!({
            "type": "user",
            "message": { "role": "user", "content": text }
        })
        .to_string()
    }

    fn user_array(blocks: serde_json::Value) -> String {
        serde_json::json!({
            "type": "user",
            "message": { "role": "user", "content": blocks }
        })
        .to_string()
    }

    fn assistant_text(text: &str) -> String {
        serde_json::json!({
            "type": "assistant",
            "message": {
                "role": "assistant",
                "content": [{ "type": "text", "text": text }]
            }
        })
        .to_string()
    }

    fn summary_entry(text: &str) -> String {
        serde_json::json!({ "type": "summary", "summary": text }).to_string()
    }

    #[test]
    fn test_title_from_summary_entry() {
        let jsonl = [
            user_text("hello there"),
            summary_entry("A chat about greetings"),
            assistant_text("Hi!"),
        ]
        .join("\n");
        let resp = parse_transcript(&jsonl).unwrap();
        assert_eq!(resp.title.as_deref(), Some("A chat about greetings"));
    }

    #[test]
    fn test_title_falls_back_to_first_user_message() {
        let jsonl = [user_text("explain recursion"), assistant_text("Sure!")].join("\n");
        let resp = parse_transcript(&jsonl).unwrap();
        assert_eq!(resp.title.as_deref(), Some("explain recursion"));
    }

    #[test]
    fn test_title_truncated_to_60_chars() {
        let long = "a".repeat(80);
        let jsonl = user_text(&long);
        let resp = parse_transcript(&jsonl).unwrap();
        assert_eq!(resp.title.as_deref().map(|t| t.len()), Some(60));
    }

    #[test]
    fn test_user_string_content_is_rendered() {
        let jsonl = user_text("hello");
        let resp = parse_transcript(&jsonl).unwrap();
        assert_eq!(resp.messages.len(), 1);
        assert_eq!(resp.messages[0].role, "user");
        assert_eq!(resp.messages[0].content[0]["type"], "text");
        assert_eq!(resp.messages[0].content[0]["text"], "hello");
    }

    #[test]
    fn test_user_array_content_is_rendered() {
        let jsonl = user_array(serde_json::json!([{ "type": "text", "text": "hi" }]));
        let resp = parse_transcript(&jsonl).unwrap();
        assert_eq!(resp.messages.len(), 1);
        assert_eq!(resp.messages[0].content[0]["text"], "hi");
    }

    #[test]
    fn test_tool_result_only_user_messages_are_filtered() {
        let jsonl = user_array(
            serde_json::json!([{ "type": "tool_result", "tool_use_id": "x", "content": "ok" }]),
        );
        let resp = parse_transcript(&jsonl).unwrap();
        assert_eq!(resp.messages.len(), 0);
    }

    #[test]
    fn test_thinking_blocks_filtered_from_assistant() {
        let jsonl = serde_json::json!({
            "type": "assistant",
            "message": {
                "role": "assistant",
                "content": [
                    { "type": "thinking", "thinking": "hmm" },
                    { "type": "text", "text": "answer" }
                ]
            }
        })
        .to_string();
        let resp = parse_transcript(&jsonl).unwrap();
        assert_eq!(resp.messages.len(), 1);
        assert_eq!(resp.messages[0].content.len(), 1);
        assert_eq!(resp.messages[0].content[0]["text"], "answer");
    }

    #[test]
    fn test_invalid_lines_are_skipped() {
        let jsonl = ["not json", &user_text("valid"), "also not json"].join("\n");
        let resp = parse_transcript(&jsonl).unwrap();
        assert_eq!(resp.messages.len(), 1);
    }

    #[test]
    fn test_empty_transcript() {
        let resp = parse_transcript("").unwrap();
        assert!(resp.title.is_none());
        assert!(resp.messages.is_empty());
    }

    #[test]
    fn test_extract_title_from_jsonl_uses_summary() {
        let chunk = [summary_entry("My session"), user_text("first msg")].join("\n");
        assert_eq!(extract_title_from_jsonl(&chunk).as_deref(), Some("My session"));
    }

    #[test]
    fn test_extract_title_from_jsonl_falls_back_to_user() {
        let chunk = user_text("what is rust?");
        assert_eq!(
            extract_title_from_jsonl(&chunk).as_deref(),
            Some("what is rust?")
        );
    }
}
