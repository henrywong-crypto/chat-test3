use serde::Serialize;

#[derive(Serialize)]
pub struct QuestionAnswerPayload {
    #[serde(rename = "type")]
    pub type_: String,
    pub request_id: String,
    pub answers: serde_json::Value,
}
