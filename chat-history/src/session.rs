use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sftp_client::{open_sftp_session, DirEntry, SftpSession};
use ssh_client::connect_ssh;
use std::path::PathBuf;
use tokio::io::AsyncReadExt;

use crate::{project::find_all_project_dirs, Content};

#[derive(Serialize)]
pub struct ChatSession {
    pub session_id: String,
    pub title: String,
    pub last_active_at: DateTime<Utc>,
}

#[derive(Deserialize)]
struct JournalMessage {
    content: Content,
}

#[derive(Deserialize)]
struct JournalEntry {
    #[serde(rename = "type")]
    entry_type: String,
    message: JournalMessage,
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
        let dir_entries: Vec<DirEntry> = sftp.read_dir(project_dir).await?.collect();
        let mut chat_sessions = build_chat_sessions(&sftp, dir_entries, project_dir).await?;
        all_chat_sessions.append(&mut chat_sessions);
    }
    all_chat_sessions.sort_by(|a, b| b.last_active_at.cmp(&a.last_active_at));
    Ok(all_chat_sessions)
}

pub(crate) fn extract_last_user_title(contents: &str) -> Option<String> {
    contents
        .lines()
        .rev()
        .filter_map(|line| serde_json::from_str::<JournalEntry>(line).ok())
        .filter(|e| e.entry_type == "user")
        .find_map(|e| extract_user_title(e.message.content))
}

fn extract_user_title(content: Content) -> Option<String> {
    match content {
        Content::Text(text) => Some(text),
        Content::Blocks(blocks) => blocks.into_iter().find_map(|b| b.text),
    }
}

async fn build_chat_sessions(
    sftp: &SftpSession,
    dir_entries: Vec<DirEntry>,
    project_dir: &str,
) -> Result<Vec<ChatSession>> {
    let mut chat_sessions = Vec::new();
    for dir_entry in &dir_entries {
        let name = dir_entry.file_name();
        let Some(session_id) = name.strip_suffix(".jsonl") else {
            continue;
        };
        if session_id.starts_with("agent-") {
            continue;
        }
        chat_sessions
            .push(build_chat_session_with_title(sftp, dir_entry, session_id, project_dir).await?);
    }
    Ok(chat_sessions)
}

async fn build_chat_session_with_title(
    sftp: &SftpSession,
    dir_entry: &DirEntry,
    session_id: &str,
    project_dir: &str,
) -> Result<ChatSession> {
    let mtime = dir_entry
        .metadata()
        .mtime
        .context("missing mtime on session file")?;
    let last_active_at = DateTime::from_timestamp(mtime as i64, 0)
        .context("mtime is out of range for a timestamp")?;
    let path = format!("{project_dir}/{session_id}.jsonl");
    let title = fetch_session_title(sftp, &path)
        .await?
        .unwrap_or_else(|| session_id.to_owned());
    Ok(ChatSession {
        session_id: session_id.to_owned(),
        title,
        last_active_at,
    })
}

async fn fetch_session_title(sftp: &SftpSession, path: &str) -> Result<Option<String>> {
    let mut file = sftp.open(path).await?;
    let mut contents = String::new();
    file.read_to_string(&mut contents).await?;
    Ok(extract_last_user_title(&contents))
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE_FIRST_USER: &str = r#"{"parentUuid":null,"isSidechain":false,"userType":"external","cwd":"/home/user/project","sessionId":"00000000-0000-0000-0000-000000000001","version":"0.0.0","gitBranch":"main","type":"user","message":{"role":"user","content":"first message"},"uuid":"00000000-0000-0000-0000-000000000002","timestamp":"2020-01-01T00:00:00.000Z","todos":[],"permissionMode":"default"}"#;
    const FIXTURE_TOOL_RESULT_USER: &str = r#"{"parentUuid":"00000000-0000-0000-0000-000000000003","isSidechain":false,"userType":"external","cwd":"/home/user/project","sessionId":"00000000-0000-0000-0000-000000000001","version":"0.0.0","gitBranch":"main","slug":"dummy-slug","type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"some error","is_error":true,"tool_use_id":"tooluse_dummy"}]},"uuid":"00000000-0000-0000-0000-000000000004","timestamp":"2020-01-01T00:00:01.000Z","toolUseResult":"some error","sourceToolAssistantUUID":"00000000-0000-0000-0000-000000000003"}"#;
    const FIXTURE_LAST_USER: &str = r#"{"parentUuid":"00000000-0000-0000-0000-000000000005","isSidechain":false,"userType":"external","cwd":"/home/user/project","sessionId":"00000000-0000-0000-0000-000000000001","version":"0.0.0","gitBranch":"main","slug":"dummy-slug","type":"user","message":{"role":"user","content":"last message"},"uuid":"00000000-0000-0000-0000-000000000006","timestamp":"2020-01-01T00:00:02.000Z","todos":[],"permissionMode":"plan"}"#;

    #[test]
    fn test_title_is_last_user_message() {
        let jsonl = [
            FIXTURE_FIRST_USER,
            FIXTURE_TOOL_RESULT_USER,
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
}
