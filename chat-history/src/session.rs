use serde::Deserialize;

use crate::Content;

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

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE_FIRST_USER: &str = r#"{"parentUuid":null,"isSidechain":false,"userType":"external","cwd":"/home/user/project","sessionId":"00000000-0000-0000-0000-000000000001","version":"0.0.0","gitBranch":"main","type":"user","message":{"role":"user","content":"first message"},"uuid":"00000000-0000-0000-0000-000000000002","timestamp":"2020-01-01T00:00:00.000Z","todos":[],"permissionMode":"default"}"#;
    const FIXTURE_TOOL_RESULT_USER: &str = r#"{"parentUuid":"00000000-0000-0000-0000-000000000003","isSidechain":false,"userType":"external","cwd":"/home/user/project","sessionId":"00000000-0000-0000-0000-000000000001","version":"0.0.0","gitBranch":"main","slug":"dummy-slug","type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"some error","is_error":true,"tool_use_id":"tooluse_dummy"}]},"uuid":"00000000-0000-0000-0000-000000000004","timestamp":"2020-01-01T00:00:01.000Z","toolUseResult":"some error","sourceToolAssistantUUID":"00000000-0000-0000-0000-000000000003"}"#;
    const FIXTURE_LAST_USER: &str = r#"{"parentUuid":"00000000-0000-0000-0000-000000000005","isSidechain":false,"userType":"external","cwd":"/home/user/project","sessionId":"00000000-0000-0000-0000-000000000001","version":"0.0.0","gitBranch":"main","slug":"dummy-slug","type":"user","message":{"role":"user","content":"last message"},"uuid":"00000000-0000-0000-0000-000000000006","timestamp":"2020-01-01T00:00:02.000Z","todos":[],"permissionMode":"plan"}"#;

    #[test]
    fn test_title_is_last_user_message() {
        let jsonl = [FIXTURE_FIRST_USER, FIXTURE_TOOL_RESULT_USER, FIXTURE_LAST_USER].join("\n");
        assert_eq!(extract_last_user_title(&jsonl).as_deref(), Some("last message"));
    }

    #[test]
    fn test_title_skips_tool_result_user_entries() {
        let jsonl = [FIXTURE_FIRST_USER, FIXTURE_TOOL_RESULT_USER].join("\n");
        assert_eq!(extract_last_user_title(&jsonl).as_deref(), Some("first message"));
    }

    #[test]
    fn test_title_returns_none_for_empty_chat_history() {
        assert_eq!(extract_last_user_title(""), None);
    }
}
