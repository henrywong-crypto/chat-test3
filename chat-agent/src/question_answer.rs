use anyhow::Result;

pub fn build_question_answer_payload(
    request_id: &str,
    answers: &serde_json::Value,
) -> Result<String> {
    Ok(serde_json::to_string(&serde_json::json!({
        "type": "answer_question",
        "request_id": request_id,
        "answers": answers,
    }))?)
}
