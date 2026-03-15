use anyhow::Result;

pub fn build_hello_payload(task_id: &str) -> Result<String> {
    Ok(serde_json::to_string(&serde_json::json!({
        "type": "hello",
        "task_id": task_id,
    }))?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(json: &str) -> serde_json::Value {
        serde_json::from_str(json).expect("invalid JSON")
    }

    #[test]
    fn test_hello_type_field() {
        let json = build_hello_payload("task-abc").unwrap();
        assert_eq!(parse(&json)["type"], "hello");
    }

    #[test]
    fn test_hello_task_id_field() {
        let json = build_hello_payload("task-abc").unwrap();
        assert_eq!(parse(&json)["task_id"], "task-abc");
    }
}
