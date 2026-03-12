use serde::Deserialize;

use crate::{ChatHistory, ChatMessage, Content};

#[derive(Deserialize)]
struct JournalMessage {
    role: Option<String>,
    content: Content,
}

#[derive(Deserialize)]
struct JournalEntry {
    #[serde(rename = "type")]
    entry_type: String,
    message: JournalMessage,
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

fn build_chat_message(entry: JournalEntry) -> Option<ChatMessage> {
    let role = entry.message.role.unwrap_or(entry.entry_type);
    Some(ChatMessage { role, content: entry.message.content })
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
        let chat_history = parse_chat_history(FIXTURE_FIRST_USER);
        assert_eq!(chat_history.messages.len(), 1);
        assert_eq!(chat_history.messages[0].role, "user");
        let Content::Text(ref text) = chat_history.messages[0].content else { panic!() };
        assert_eq!(text, "first message");
    }

    #[test]
    fn test_tool_result_user_messages_are_included() {
        let chat_history = parse_chat_history(FIXTURE_TOOL_RESULT_USER);
        assert_eq!(chat_history.messages.len(), 1);
        let Content::Blocks(ref blocks) = chat_history.messages[0].content else { panic!() };
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
        let Content::Blocks(ref blocks) = chat_history.messages[0].content else { panic!() };
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
