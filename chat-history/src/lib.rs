use anyhow::{Context, Result};
use chrono::{DateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use sftp_client::{open_sftp_session, DirEntry, SftpSession};
use ssh_client::connect_ssh;
use std::path::PathBuf;
use tokio::io::AsyncReadExt;

mod jsonl;

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
    pub content: Content,
}

#[derive(Deserialize, Serialize)]
#[serde(untagged)]
pub enum Content {
    Text(String),
    Blocks(Vec<ContentBlock>),
}

#[derive(Deserialize, Serialize)]
pub struct ContentBlock {
    #[serde(rename = "type")]
    pub block_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(flatten)]
    fields: serde_json::Map<String, serde_json::Value>,
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
        let dir_entries: Vec<DirEntry> = sftp.read_dir(project_dir).await?.collect();
        let mut chat_sessions = build_chat_sessions(&sftp, dir_entries, project_dir).await?;
        all_chat_sessions.append(&mut chat_sessions);
    }
    all_chat_sessions.sort_by(|a, b| b.last_active_at.cmp(&a.last_active_at));
    Ok(all_chat_sessions)
}

async fn find_all_project_dirs(sftp: &SftpSession, ssh_user_home: &str) -> Vec<String> {
    let projects_base = projects_base_path(ssh_user_home);
    // Directory may not exist yet on a fresh VM; treat as empty rather than an error
    let top_entries: Vec<DirEntry> = sftp.read_dir(&projects_base).await
        .map(|entries| entries.collect())
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
        let name = dir_entry.file_name();
        let Some(session_id) = name.strip_suffix(".jsonl") else { continue };
        if session_id.starts_with("agent-") {
            continue;
        }
        chat_sessions.push(build_chat_session_with_title(sftp, dir_entry, session_id, project_dir).await?);
    }
    Ok(chat_sessions)
}

async fn build_chat_session_with_title(
    sftp: &SftpSession,
    dir_entry: &DirEntry,
    session_id: &str,
    project_dir: &str,
) -> Result<ChatSession> {
    let mtime = dir_entry.metadata().mtime.context("missing mtime on session file")?;
    let last_active_at = Utc
        .timestamp_opt(mtime as i64, 0)
        .single()
        .context("mtime is out of range for a timestamp")?;
    let path = format!("{project_dir}/{session_id}.jsonl");
    let title = fetch_session_title(sftp, &path).await?.unwrap_or_else(|| session_id.to_owned());
    Ok(ChatSession { session_id: session_id.to_owned(), title, last_active_at })
}

async fn fetch_session_title(sftp: &SftpSession, path: &str) -> Result<Option<String>> {
    let mut file = sftp.open(path).await?;
    let mut contents = String::new();
    file.read_to_string(&mut contents).await?;
    Ok(jsonl::extract_last_user_title(&contents))
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
    let mut chat_history_path = None;
    for dir in &project_dirs {
        let path = format!("{dir}/{session_id}.jsonl");
        if sftp.open(&path).await.is_ok() {
            chat_history_path = Some(path);
            break;
        }
    }
    let chat_history_path =
        chat_history_path.context(format!("session not found: {session_id}"))?;
    let mut file = sftp.open(&chat_history_path).await?;
    let mut contents = String::new();
    file.read_to_string(&mut contents).await?;
    Ok(jsonl::parse_chat_history(&contents))
}
