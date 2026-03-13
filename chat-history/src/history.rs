use anyhow::{Context, Result};
use serde::Serialize;
use sftp_client::open_sftp_session;
use ssh_client::connect_ssh;
use std::path::PathBuf;
use tokio::io::AsyncReadExt;

use crate::{journal::JournalEntry, project::find_all_project_dirs, Content};

#[derive(Serialize)]
pub struct ChatHistory {
    pub messages: Vec<ChatMessage>,
}

#[derive(Serialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: Content,
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
    Ok(parse_chat_history(&contents))
}

pub(crate) fn parse_chat_history(contents: &str) -> ChatHistory {
    let messages = contents
        .lines()
        .filter_map(|line| serde_json::from_str::<JournalEntry>(line).ok())
        .filter(|e| matches!(e.entry_type.as_str(), "user" | "assistant"))
        .filter_map(build_chat_message)
        .collect();
    ChatHistory { messages }
}

fn build_chat_message(entry: JournalEntry) -> Option<ChatMessage> {
    let role = entry.message.role.unwrap_or(entry.entry_type);
    Some(ChatMessage {
        role,
        content: entry.message.content,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE_FIRST_USER: &str = r#"{"parentUuid":null,"isSidechain":false,"userType":"external","cwd":"/home/user/project","sessionId":"00000000-0000-0000-0000-000000000001","version":"0.0.0","gitBranch":"main","type":"user","message":{"role":"user","content":"first message"},"uuid":"00000000-0000-0000-0000-000000000002","timestamp":"2020-01-01T00:00:00.000Z","todos":[],"permissionMode":"default"}"#;
    const FIXTURE_TOOL_RESULT_USER: &str = r#"{"parentUuid":"00000000-0000-0000-0000-000000000003","isSidechain":false,"userType":"external","cwd":"/home/user/project","sessionId":"00000000-0000-0000-0000-000000000001","version":"0.0.0","gitBranch":"main","slug":"dummy-slug","type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"some error","is_error":true,"tool_use_id":"tooluse_dummy"}]},"uuid":"00000000-0000-0000-0000-000000000004","timestamp":"2020-01-01T00:00:01.000Z","toolUseResult":"some error","sourceToolAssistantUUID":"00000000-0000-0000-0000-000000000003"}"#;

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
    fn test_user_string_content_is_rendered() {
        let chat_history = parse_chat_history(FIXTURE_FIRST_USER);
        assert_eq!(chat_history.messages.len(), 1);
        assert_eq!(chat_history.messages[0].role, "user");
        let Content::Text(ref text) = chat_history.messages[0].content else {
            panic!()
        };
        assert_eq!(text, "first message");
    }

    #[test]
    fn test_tool_result_user_messages_are_included() {
        let chat_history = parse_chat_history(FIXTURE_TOOL_RESULT_USER);
        assert_eq!(chat_history.messages.len(), 1);
        let Content::Blocks(ref blocks) = chat_history.messages[0].content else {
            panic!()
        };
        assert_eq!(blocks[0].block_type, "tool_result");
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
        let chat_history = parse_chat_history(&jsonl);
        assert_eq!(chat_history.messages.len(), 1);
        let Content::Blocks(ref blocks) = chat_history.messages[0].content else {
            panic!()
        };
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].block_type, "thinking");
        assert_eq!(blocks[1].text.as_deref(), Some("answer"));
    }

    #[test]
    fn test_invalid_lines_are_skipped() {
        let jsonl = ["not json", FIXTURE_FIRST_USER, "also not json"].join("\n");
        let chat_history = parse_chat_history(&jsonl);
        assert_eq!(chat_history.messages.len(), 1);
    }

    #[test]
    fn test_empty_chat_history() {
        let chat_history = parse_chat_history("");
        assert!(chat_history.messages.is_empty());
    }
}
