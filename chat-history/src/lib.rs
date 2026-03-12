use anyhow::{anyhow, Result};
use chrono::{DateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use sftp_client::{DirEntry, SftpSession};
use std::path::PathBuf;
use tokio::io::AsyncReadExt;

use sftp_client::open_sftp_session;
use ssh_client::connect_ssh;

#[derive(Serialize)]
pub struct ChatSession {
    pub session_id: String,
    pub title: String,
    pub last_active_at: DateTime<Utc>,
}

#[derive(Serialize)]
pub struct ChatHistory {
    pub messages: Vec<ChatMessage>,
}

#[derive(Serialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: Vec<ContentBlock>,
}

#[derive(Deserialize, Serialize)]
pub struct ContentBlock {
    #[serde(rename = "type")]
    block_type: String,
    #[serde(flatten)]
    fields: serde_json::Map<String, serde_json::Value>,
}

impl ContentBlock {
    fn from_text(text: String) -> ContentBlock {
        let mut fields = serde_json::Map::new();
        fields.insert("text".to_owned(), serde_json::Value::String(text));
        ContentBlock { block_type: "text".to_owned(), fields }
    }

    fn text(&self) -> Option<&str> {
        self.fields.get("text").and_then(|v| v.as_str())
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Content {
    Text(String),
    Blocks(Vec<ContentBlock>),
}

#[derive(Deserialize)]
struct RawMessage {
    role: Option<String>,
    content: Content,
}

#[derive(Deserialize)]
struct JournalEntry {
    #[serde(rename = "type")]
    entry_type: String,
    message: RawMessage,
}

fn projects_base_path(ssh_user_home: &str) -> String {
    format!("{ssh_user_home}/.claude/projects")
}

pub async fn list_chat_sessions(
    guest_ip: &str,
    ssh_key_path: &PathBuf,
    ssh_user: &str,
    vm_host_key_path: &PathBuf,
    ssh_user_home: &str,
) -> Result<Vec<ChatSession>> {
    let mut ssh_handle = connect_ssh(guest_ip, ssh_key_path, ssh_user, vm_host_key_path).await?;
    let sftp = open_sftp_session(&mut ssh_handle).await?;
    let project_dirs = find_all_project_dirs(&sftp, ssh_user_home).await;
    let mut all_chat_sessions = Vec::new();
    for project_dir in &project_dirs {
        let dir_entries: Vec<DirEntry> = sftp
            .read_dir(project_dir)
            .await
            .map(|rd| rd.collect())
            .unwrap_or_default();
        let mut chat_sessions = build_chat_sessions(&sftp, dir_entries, project_dir).await?;
        all_chat_sessions.append(&mut chat_sessions);
    }
    all_chat_sessions.sort_by(|a, b| b.last_active_at.cmp(&a.last_active_at));
    Ok(all_chat_sessions)
}

async fn find_all_project_dirs(sftp: &SftpSession, ssh_user_home: &str) -> Vec<String> {
    let projects_base = projects_base_path(ssh_user_home);
    let top_entries: Vec<DirEntry> = sftp
        .read_dir(&projects_base)
        .await
        .map(|rd| rd.collect())
        .unwrap_or_default();
    let mut project_dirs = Vec::new();
    for entry in top_entries {
        let name = entry.file_name();
        if name.starts_with('.') {
            continue;
        }
        let path = format!("{projects_base}/{name}");
        if entry.file_type().is_dir() {
            project_dirs.push(path);
        }
    }
    project_dirs
}

async fn build_chat_sessions(
    sftp: &SftpSession,
    dir_entries: Vec<DirEntry>,
    project_dir: &str,
) -> Result<Vec<ChatSession>> {
    let mut chat_sessions = Vec::new();
    for dir_entry in &dir_entries {
        if let Some(chat_session) =
            build_chat_session_with_title(sftp, dir_entry, project_dir).await
        {
            chat_sessions.push(chat_session);
        }
    }
    Ok(chat_sessions)
}

async fn build_chat_session_with_title(
    sftp: &SftpSession,
    dir_entry: &DirEntry,
    project_dir: &str,
) -> Option<ChatSession> {
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
    Some(ChatSession { session_id, title, last_active_at })
}

async fn fetch_session_title(sftp: &SftpSession, path: &str) -> Option<String> {
    let mut file = sftp.open(path).await.ok()?;
    let mut contents = String::new();
    file.read_to_string(&mut contents).await.ok()?;
    extract_last_user_title(&contents)
}

fn extract_last_user_title(contents: &str) -> Option<String> {
    contents
        .lines()
        .filter_map(|line| serde_json::from_str::<JournalEntry>(line).ok())
        .filter(|e| e.entry_type == "user")
        .filter_map(|e| {
            normalize_content(e.message.content)
                .into_iter()
                .find_map(|b| b.text().map(str::to_owned))
        })
        .last()
}

pub async fn fetch_chat_history(
    guest_ip: &str,
    ssh_key_path: &PathBuf,
    ssh_user: &str,
    vm_host_key_path: &PathBuf,
    session_id: &str,
    ssh_user_home: &str,
) -> Result<ChatHistory> {
    let mut ssh_handle = connect_ssh(guest_ip, ssh_key_path, ssh_user, vm_host_key_path).await?;
    let sftp = open_sftp_session(&mut ssh_handle).await?;
    let project_dirs = find_all_project_dirs(&sftp, ssh_user_home).await;
    let chat_history_path = find_session_path(&sftp, &project_dirs, session_id)
        .await
        .ok_or_else(|| anyhow!("session not found: {session_id}"))?;
    let mut file = sftp.open(&chat_history_path).await?;
    let mut contents = String::new();
    file.read_to_string(&mut contents).await?;
    parse_chat_history(&contents)
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

fn parse_chat_history(contents: &str) -> Result<ChatHistory> {
    let messages = contents
        .lines()
        .filter_map(|line| serde_json::from_str::<JournalEntry>(line).ok())
        .filter(|e| matches!(e.entry_type.as_str(), "user" | "assistant"))
        .filter_map(build_chat_message)
        .collect();
    Ok(ChatHistory { messages })
}

fn build_chat_message(entry: JournalEntry) -> Option<ChatMessage> {
    let role = entry.message.role.unwrap_or(entry.entry_type);
    let content = normalize_content(entry.message.content);
    if content.is_empty() {
        return None;
    }
    Some(ChatMessage { role, content })
}

fn normalize_content(content: Content) -> Vec<ContentBlock> {
    match content {
        Content::Text(text) => vec![ContentBlock::from_text(text)],
        Content::Blocks(blocks) => blocks,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE_FIRST_USER: &str = r#"{"parentUuid":null,"isSidechain":false,"userType":"external","cwd":"/home/user/project","sessionId":"00000000-0000-0000-0000-000000000001","version":"0.0.0","gitBranch":"main","type":"user","message":{"role":"user","content":"first message"},"uuid":"00000000-0000-0000-0000-000000000002","timestamp":"2020-01-01T00:00:00.000Z","todos":[],"permissionMode":"default"}"#;
    const FIXTURE_TOOL_RESULT_USER: &str = r#"{"parentUuid":"00000000-0000-0000-0000-000000000003","isSidechain":false,"userType":"external","cwd":"/home/user/project","sessionId":"00000000-0000-0000-0000-000000000001","version":"0.0.0","gitBranch":"main","slug":"dummy-slug","type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"some error","is_error":true,"tool_use_id":"tooluse_dummy"}]},"uuid":"00000000-0000-0000-0000-000000000004","timestamp":"2020-01-01T00:00:01.000Z","toolUseResult":"some error","sourceToolAssistantUUID":"00000000-0000-0000-0000-000000000003"}"#;
    const FIXTURE_LAST_USER: &str = r#"{"parentUuid":"00000000-0000-0000-0000-000000000005","isSidechain":false,"userType":"external","cwd":"/home/user/project","sessionId":"00000000-0000-0000-0000-000000000001","version":"0.0.0","gitBranch":"main","slug":"dummy-slug","type":"user","message":{"role":"user","content":"last message"},"uuid":"00000000-0000-0000-0000-000000000006","timestamp":"2020-01-01T00:00:02.000Z","todos":[],"permissionMode":"plan"}"#;

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
            Some("last message")
        );
    }

    #[test]
    fn test_title_skips_tool_result_user_entries() {
        let jsonl = [FIXTURE_FIRST_USER, FIXTURE_TOOL_RESULT_USER].join("\n");
        assert_eq!(
            extract_last_user_title(&jsonl).as_deref(),
            Some("first message")
        );
    }

    #[test]
    fn test_title_returns_none_for_empty_chat_history() {
        assert_eq!(extract_last_user_title(""), None);
    }

    #[test]
    fn test_user_string_content_is_rendered() {
        let chat_history = parse_chat_history(FIXTURE_FIRST_USER).unwrap();
        assert_eq!(chat_history.messages.len(), 1);
        assert_eq!(chat_history.messages[0].role, "user");
        assert_eq!(chat_history.messages[0].content[0].block_type, "text");
        assert_eq!(chat_history.messages[0].content[0].text(), Some("first message"));
    }

    #[test]
    fn test_tool_result_user_messages_are_included() {
        let chat_history = parse_chat_history(FIXTURE_TOOL_RESULT_USER).unwrap();
        assert_eq!(chat_history.messages.len(), 1);
        assert_eq!(chat_history.messages[0].content[0].block_type, "tool_result");
    }

    #[test]
    fn test_thinking_blocks_included_in_assistant() {
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
        let chat_history = parse_chat_history(&jsonl).unwrap();
        assert_eq!(chat_history.messages.len(), 1);
        assert_eq!(chat_history.messages[0].content.len(), 2);
        assert_eq!(chat_history.messages[0].content[0].block_type, "thinking");
        assert_eq!(chat_history.messages[0].content[1].text(), Some("answer"));
    }

    #[test]
    fn test_invalid_lines_are_skipped() {
        let jsonl = ["not json", FIXTURE_FIRST_USER, "also not json"].join("\n");
        let chat_history = parse_chat_history(&jsonl).unwrap();
        assert_eq!(chat_history.messages.len(), 1);
    }

    #[test]
    fn test_empty_chat_history() {
        let chat_history = parse_chat_history("").unwrap();
        assert!(chat_history.messages.is_empty());
    }
}
