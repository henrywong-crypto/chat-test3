use anyhow::Result;
use serde::Serialize;

#[derive(Serialize)]
struct QuestionAnswerPayload<'a> {
    #[serde(rename = "type")]
    type_: &'a str,
    request_id: &'a str,
    answers: &'a serde_json::Value,
}

pub fn build_question_answer_payload(
    request_id: &str,
    answers: &serde_json::Value,
) -> Result<String> {
    let question_answer_payload = QuestionAnswerPayload {
        type_: "answer_question",
        request_id,
        answers,
    };
    Ok(serde_json::to_string(&question_answer_payload)?)
}
