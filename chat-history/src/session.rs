use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use russh_sftp::client::{fs::DirEntry, SftpSession};
use serde::Serialize;
use sftp_client::open_sftp_session;
use ssh_client::connect_ssh;
use std::path::PathBuf;
use tokio::io::AsyncReadExt;

use crate::{journal::JournalEntry, project::find_all_project_dirs, Content};

#[derive(Serialize)]
pub struct ChatSession {
    pub session_id: String,
    pub project_dir: String,
    pub title: String,
    pub last_active_at: DateTime<Utc>,
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
        for dir_entry in &dir_entries {
            let name = dir_entry.file_name();
            let Some(session_id) = name.strip_suffix(".jsonl") else {
                continue;
            };
            if let Some(chat_session) =
                build_chat_session_with_title(&sftp, dir_entry, session_id, project_dir).await?
            {
                all_chat_sessions.push(chat_session);
            }
        }
    }
    all_chat_sessions.sort_by(|a, b| b.last_active_at.cmp(&a.last_active_at));
    Ok(all_chat_sessions)
}

pub async fn delete_chat_session(
    guest_ip: &str,
    ssh_key_path: &PathBuf,
    ssh_user: &str,
    vm_host_key_path: &PathBuf,
    session_id: &str,
    project_dir: &str,
) -> Result<()> {
    let mut ssh_handle = connect_ssh(guest_ip, ssh_key_path, ssh_user, vm_host_key_path).await?;
    let sftp = open_sftp_session(&mut ssh_handle).await?;
    let path = format!("{project_dir}/{session_id}.jsonl");
    sftp.remove_file(&path).await?;
    delete_session_dir(&sftp, project_dir, session_id).await
}

async fn delete_session_dir(sftp: &SftpSession, project_dir: &str, session_id: &str) -> Result<()> {
    let dir_path = format!("{project_dir}/{session_id}");
    match sftp.try_exists(&dir_path).await {
        Ok(true) => remove_dir_all(sftp, &dir_path, 2).await,
        _ => Ok(()),
    }
}

fn remove_dir_all<'a>(
    sftp: &'a SftpSession,
    path: &'a str,
    max_depth: usize,
) -> BoxFuture<'a, Result<()>> {
    Box::pin(async move {
        let entries: Vec<DirEntry> = sftp.read_dir(path).await?.collect();
        for entry in &entries {
            let entry_path = format!("{path}/{}", entry.file_name());
            if entry.file_type().is_dir() {
                if max_depth == 0 {
                    continue;
                }
                remove_dir_all(sftp, &entry_path, max_depth - 1).await?;
            } else {
                sftp.remove_file(&entry_path).await?;
            }
        }
        sftp.remove_dir(path).await?;
        Ok(())
    })
}

pub(crate) fn extract_last_user_title(contents: &str) -> Option<String> {
    contents
        .lines()
        .rev()
        .filter_map(|line| serde_json::from_str::<JournalEntry>(line).ok())
        .filter(|e| e.type_ == "user")
        .filter(|e| !e.is_meta)
        .filter(|e| !e.is_compact_summary)
        .find_map(|e| extract_user_title(e.message.content))
}

fn extract_user_title(content: Content) -> Option<String> {
    match content {
        Content::Text(text)
            // Matches all <local-command-*> tags (e.g. <local-command-stdout>). The
            // <local-command-caveat> entries are already excluded via is_meta, but
            // other variants like <local-command-stdout> lack isMeta so need this check.
            if !text.starts_with("<command-name>") && !text.starts_with("<local-command-") =>
        {
            Some(text)
        }
        Content::Text(_) => None,
        Content::ContentBlocks(blocks) => blocks.into_iter().find_map(|b| b.text),
    }
}

async fn build_chat_session_with_title(
    sftp: &SftpSession,
    dir_entry: &DirEntry,
    session_id: &str,
    project_dir: &str,
) -> Result<Option<ChatSession>> {
    let mtime = dir_entry
        .metadata()
        .mtime
        .context("missing mtime on session file")?;
    let last_active_at = DateTime::from_timestamp(mtime as i64, 0)
        .context("mtime is out of range for a timestamp")?;
    let path = format!("{project_dir}/{session_id}.jsonl");
    let Some(title) = fetch_session_title(sftp, &path).await? else {
        return Ok(None);
    };
    Ok(Some(ChatSession {
        session_id: session_id.to_owned(),
        project_dir: project_dir.to_owned(),
        title,
        last_active_at,
    }))
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
    const FIXTURE_IS_META_USER: &str = r#"{"parentUuid":null,"isSidechain":false,"userType":"external","cwd":"/home/user/project","sessionId":"00000000-0000-0000-0000-000000000001","version":"0.0.0","gitBranch":"main","type":"user","isMeta":true,"message":{"role":"user","content":"<local-command-caveat>Caveat: The messages below were generated by the user while running local commands.</local-command-caveat>"},"uuid":"00000000-0000-0000-0000-000000000002","timestamp":"2020-01-01T00:00:00.000Z"}"#;
    const FIXTURE_SLASH_COMMAND_USER: &str = r#"{"parentUuid":null,"isSidechain":false,"userType":"external","cwd":"/home/user/project","sessionId":"00000000-0000-0000-0000-000000000001","version":"0.0.0","gitBranch":"main","type":"user","message":{"role":"user","content":"<command-name>/clear</command-name>\n            <command-message>clear</command-message>\n            <command-args></command-args>"},"uuid":"00000000-0000-0000-0000-000000000003","timestamp":"2020-01-01T00:00:01.000Z"}"#;

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

    #[test]
    fn test_title_skips_is_meta_entries() {
        let jsonl = [FIXTURE_IS_META_USER, FIXTURE_FIRST_USER].join("\n");
        assert_eq!(
            extract_last_user_title(&jsonl).as_deref(),
            Some("first message")
        );
    }

    #[test]
    fn test_title_skips_slash_command_entries() {
        let jsonl = [FIXTURE_FIRST_USER, FIXTURE_SLASH_COMMAND_USER].join("\n");
        assert_eq!(
            extract_last_user_title(&jsonl).as_deref(),
            Some("first message")
        );
    }

    #[test]
    fn test_title_skips_compact_summary_entries() {
        let compact_summary = serde_json::json!({
            "type": "user",
            "isCompactSummary": true,
            "message": { "role": "user", "content": "This session is being continued.\n\nSummary:\nThe user asked about widgets." }
        })
        .to_string();
        let jsonl = [FIXTURE_FIRST_USER, &compact_summary].join("\n");
        assert_eq!(
            extract_last_user_title(&jsonl).as_deref(),
            Some("first message")
        );
    }

    #[test]
    fn test_title_skips_local_command_stdout_entries() {
        let local_cmd = serde_json::json!({
            "type": "user",
            "message": { "role": "user", "content": "<local-command-stdout>Set model to Default</local-command-stdout>" }
        })
        .to_string();
        let jsonl = [FIXTURE_FIRST_USER, &local_cmd].join("\n");
        assert_eq!(
            extract_last_user_title(&jsonl).as_deref(),
            Some("first message")
        );
    }
}
