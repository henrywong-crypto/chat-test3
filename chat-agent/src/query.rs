use anyhow::Result;
use serde::Serialize;

#[derive(Serialize)]
struct QueryPayload<'a> {
    #[serde(rename = "type")]
    type_: &'a str,
    task_id: &'a str,
    content: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    session_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    work_dir: Option<&'a str>,
}

pub fn build_query_payload(
    task_id: &str,
    content: &str,
    session_id: Option<&str>,
    work_dir: Option<&str>,
) -> Result<String> {
    let query_payload = QueryPayload {
        type_: "query",
        task_id,
        content,
        session_id,
        work_dir,
    };
    Ok(serde_json::to_string(&query_payload)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(json: &str) -> serde_json::Value {
        serde_json::from_str(json).expect("invalid JSON")
    }

    #[test]
    fn test_query_type_field() {
        let json = build_query_payload("task-1", "hello", None, None).unwrap();
        assert_eq!(parse(&json)["type"], "query");
    }

    #[test]
    fn test_content_field_is_present() {
        let json = build_query_payload("task-1", "hello world", None, None).unwrap();
        assert_eq!(parse(&json)["content"], "hello world");
    }

    #[test]
    fn test_session_id_included_when_some() {
        let json = build_query_payload("task-1", "hello", Some("abc-123"), None).unwrap();
        assert_eq!(parse(&json)["session_id"], "abc-123");
    }

    #[test]
    fn test_session_id_omitted_when_none() {
        let json = build_query_payload("task-1", "hello", None, None).unwrap();
        assert!(parse(&json).get("session_id").is_none());
    }

    #[test]
    fn test_special_characters_in_content_are_escaped() {
        let json =
            build_query_payload("task-1", "say \"hello\"\nand\\goodbye", None, None).unwrap();
        assert_eq!(parse(&json)["content"], "say \"hello\"\nand\\goodbye");
    }

    #[test]
    fn test_empty_content() {
        let json = build_query_payload("task-1", "", None, None).unwrap();
        assert_eq!(parse(&json)["content"], "");
    }

    #[test]
    fn test_task_id_included_in_query() {
        let json = build_query_payload("my-task-id", "hello", None, None).unwrap();
        assert_eq!(parse(&json)["task_id"], "my-task-id");
    }

    #[test]
    fn test_work_dir_included_when_some() {
        let json = build_query_payload("task-1", "hello", None, Some("/home/ubuntu")).unwrap();
        assert_eq!(parse(&json)["work_dir"], "/home/ubuntu");
    }

    #[test]
    fn test_work_dir_omitted_when_none() {
        let json = build_query_payload("task-1", "hello", None, None).unwrap();
        assert!(parse(&json).get("work_dir").is_none());
    }
}
