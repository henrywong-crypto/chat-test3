use serde::Serialize;

#[derive(Serialize)]
pub struct QueryPayload {
    #[serde(rename = "type")]
    pub type_: String,
    pub task_id: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub work_dir: Option<String>,
}
