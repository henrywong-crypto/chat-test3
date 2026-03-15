use anyhow::Result;

pub fn build_interrupt_payload(task_id: &str) -> Result<String> {
    Ok(serde_json::to_string(&serde_json::json!({
        "type": "interrupt",
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
    fn test_interrupt_type_field() {
        let json = build_interrupt_payload("abc-123").unwrap();
        assert_eq!(parse(&json)["type"], "interrupt");
    }

    #[test]
    fn test_interrupt_task_id_field() {
        let json = build_interrupt_payload("abc-123").unwrap();
        assert_eq!(parse(&json)["task_id"], "abc-123");
    }
}
