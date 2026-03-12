use anyhow::{anyhow, Result};
use chrono::{DateTime, TimeZone, Utc};
use serde::Serialize;
use sftp_client::{DirEntry, SftpSession};
use std::path::PathBuf;
use tokio::io::AsyncReadExt;

use sftp_client::open_sftp_session;
use ssh_client::connect_ssh;

#[derive(Serialize)]
pub struct SessionEntry {
    pub session_id: String,
    pub title: String,
    pub last_active_at: DateTime<Utc>,
}

#[derive(Serialize)]
pub struct TranscriptResponse {
    pub messages: Vec<TranscriptMessage>,
}

#[derive(Serialize)]
pub struct TranscriptMessage {
    pub role: String,
    pub content: Vec<serde_json::Value>,
}

const PROJECTS_BASE: &str = "/home/ubuntu/.claude/projects";

pub async fn list_sessions(
    guest_ip: &str,
    ssh_key_path: &PathBuf,
    ssh_user: &str,
    vm_host_key_path: &PathBuf,
) -> Result<Vec<SessionEntry>> {
    let mut ssh_handle = connect_ssh(guest_ip, ssh_key_path, ssh_user, vm_host_key_path).await?;
    let sftp = open_sftp_session(&mut ssh_handle).await?;
    let project_dirs = find_all_project_dirs(&sftp).await;
    let mut all_session_entries = Vec::new();
    for project_dir in &project_dirs {
        let dir_entries: Vec<DirEntry> = sftp
            .read_dir(project_dir)
            .await
            .map(|rd| rd.collect())
            .unwrap_or_default();
        let mut session_entries = build_session_entries(&sftp, dir_entries, project_dir).await?;
        all_session_entries.append(&mut session_entries);
    }
    all_session_entries.sort_by(|a, b| b.last_active_at.cmp(&a.last_active_at));
    Ok(all_session_entries)
}

async fn find_all_project_dirs(sftp: &SftpSession) -> Vec<String> {
    let top_entries: Vec<DirEntry> = sftp
        .read_dir(PROJECTS_BASE)
        .await
        .map(|rd| rd.collect())
        .unwrap_or_default();
    let mut project_dirs = Vec::new();
    for entry in top_entries {
        let name = entry.file_name();
        if name.starts_with('.') {
            continue;
        }
        let path = format!("{PROJECTS_BASE}/{name}");
        if is_directory_entry(&entry) {
            project_dirs.push(path);
        }
    }
    project_dirs
}

fn is_directory_entry(entry: &DirEntry) -> bool {
    // Unix directory bit: mode & 0o170000 == 0o040000
    entry
        .metadata()
        .permissions
        .map(|p| p & 0o170_000 == 0o040_000)
        .unwrap_or(false)
}

async fn build_session_entries(
    sftp: &SftpSession,
    dir_entries: Vec<DirEntry>,
    project_dir: &str,
) -> Result<Vec<SessionEntry>> {
    let mut session_entries = Vec::new();
    for dir_entry in &dir_entries {
        if let Some(session_entry) =
            build_session_entry_with_title(sftp, dir_entry, project_dir).await
        {
            session_entries.push(session_entry);
        }
    }
    Ok(session_entries)
}

async fn build_session_entry_with_title(
    sftp: &SftpSession,
    dir_entry: &DirEntry,
    project_dir: &str,
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
    let path = format!("{project_dir}/{session_id}.jsonl");
    let title = fetch_session_title(sftp, &path)
        .await
        .unwrap_or_else(|| session_id.clone());
    Some(SessionEntry {
        session_id,
        title,
        last_active_at,
    })
}

async fn fetch_session_title(sftp: &SftpSession, path: &str) -> Option<String> {
    let mut file = sftp.open(path).await.ok()?;
    let mut contents = String::new();
    file.read_to_string(&mut contents).await.ok()?;
    extract_last_user_title(&contents)
}

fn extract_last_user_title(contents: &str) -> Option<String> {
    let mut title = None;
    for line in contents.lines() {
        let Ok(entry) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if entry["type"].as_str() != Some("user") {
            continue;
        }
        if let Some(msg) = extract_transcript_message(&entry, "user") {
            if let Some(text) = msg.content.iter().find_map(|b| b["text"].as_str()) {
                title = Some(text.to_owned());
            }
        }
    }
    title
}

pub async fn fetch_transcript(
    guest_ip: &str,
    ssh_key_path: &PathBuf,
    ssh_user: &str,
    vm_host_key_path: &PathBuf,
    session_id: &str,
) -> Result<TranscriptResponse> {
    let mut ssh_handle = connect_ssh(guest_ip, ssh_key_path, ssh_user, vm_host_key_path).await?;
    let sftp = open_sftp_session(&mut ssh_handle).await?;
    let project_dirs = find_all_project_dirs(&sftp).await;
    let transcript_path = find_session_path(&sftp, &project_dirs, session_id)
        .await
        .ok_or_else(|| anyhow!("session not found: {session_id}"))?;
    let mut file = sftp.open(&transcript_path).await?;
    let mut contents = String::new();
    file.read_to_string(&mut contents).await?;
    parse_transcript(&contents)
}

async fn find_session_path(
    sftp: &SftpSession,
    project_dirs: &[String],
    session_id: &str,
) -> Option<String> {
    for dir in project_dirs {
        let path = format!("{dir}/{session_id}.jsonl");
        if sftp.open(&path).await.is_ok() {
            return Some(path);
        }
    }
    None
}

fn parse_transcript(contents: &str) -> Result<TranscriptResponse> {
    let mut messages = Vec::new();
    for line in contents.lines() {
        let Ok(entry) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let type_str = entry["type"].as_str().unwrap_or("");
        if matches!(type_str, "user" | "assistant") {
            if let Some(transcript_message) = extract_transcript_message(&entry, type_str) {
                messages.push(transcript_message);
            }
        }
    }
    Ok(TranscriptResponse { messages })
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
    if type_str == "user"
        && content
            .iter()
            .all(|b| b["type"].as_str() == Some("tool_result"))
    {
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

    const FIXTURE_FIRST_USER: &str = r#"{"parentUuid":null,"isSidechain":false,"userType":"external","cwd":"/home/user/project","sessionId":"00000000-0000-0000-0000-000000000001","version":"0.0.0","gitBranch":"main","type":"user","message":{"role":"user","content":"convert to argo cd"},"uuid":"00000000-0000-0000-0000-000000000002","timestamp":"2020-01-01T00:00:00.000Z","todos":[],"permissionMode":"default"}"#;
    const FIXTURE_TOOL_RESULT_USER: &str = r#"{"parentUuid":"00000000-0000-0000-0000-000000000003","isSidechain":false,"userType":"external","cwd":"/home/user/project","sessionId":"00000000-0000-0000-0000-000000000001","version":"0.0.0","gitBranch":"main","slug":"dummy-slug","type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"some error","is_error":true,"tool_use_id":"tooluse_dummy"}]},"uuid":"00000000-0000-0000-0000-000000000004","timestamp":"2020-01-01T00:00:01.000Z","toolUseResult":"some error","sourceToolAssistantUUID":"00000000-0000-0000-0000-000000000003"}"#;
    const FIXTURE_LAST_USER: &str = r#"{"parentUuid":"00000000-0000-0000-0000-000000000005","isSidechain":false,"userType":"external","cwd":"/home/user/project","sessionId":"00000000-0000-0000-0000-000000000001","version":"0.0.0","gitBranch":"main","slug":"dummy-slug","type":"user","message":{"role":"user","content":"not plan first?"},"uuid":"00000000-0000-0000-0000-000000000006","timestamp":"2020-01-01T00:00:02.000Z","todos":[],"permissionMode":"plan"}"#;

    fn make_assistant_line(text: &str) -> String {
        serde_json::json!({
            "type": "assistant",
            "message": {
                "role": "assistant",
                "content": [{ "type": "text", "text": text }]
            }
        })
        .to_string()
    }

    #[test]
    fn test_title_is_last_user_message() {
        let jsonl = [
            FIXTURE_FIRST_USER,
            FIXTURE_TOOL_RESULT_USER,
            &make_assistant_line("Sure!"),
            FIXTURE_LAST_USER,
        ]
        .join("\n");
        assert_eq!(
            extract_last_user_title(&jsonl).as_deref(),
            Some("not plan first?")
        );
    }

    #[test]
    fn test_title_skips_tool_result_only_user_entries() {
        let jsonl = [FIXTURE_FIRST_USER, FIXTURE_TOOL_RESULT_USER].join("\n");
        assert_eq!(
            extract_last_user_title(&jsonl).as_deref(),
            Some("convert to argo cd")
        );
    }

    #[test]
    fn test_title_returns_none_for_empty_transcript() {
        assert_eq!(extract_last_user_title(""), None);
    }

    #[test]
    fn test_user_string_content_is_rendered() {
        let resp = parse_transcript(FIXTURE_FIRST_USER).unwrap();
        assert_eq!(resp.messages.len(), 1);
        assert_eq!(resp.messages[0].role, "user");
        assert_eq!(resp.messages[0].content[0]["type"], "text");
        assert_eq!(resp.messages[0].content[0]["text"], "convert to argo cd");
    }

    #[test]
    fn test_tool_result_only_user_messages_are_filtered() {
        let resp = parse_transcript(FIXTURE_TOOL_RESULT_USER).unwrap();
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
        let jsonl = ["not json", FIXTURE_FIRST_USER, "also not json"].join("\n");
        let resp = parse_transcript(&jsonl).unwrap();
        assert_eq!(resp.messages.len(), 1);
    }

    #[test]
    fn test_empty_transcript() {
        let resp = parse_transcript("").unwrap();
        assert!(resp.messages.is_empty());
    }
}
