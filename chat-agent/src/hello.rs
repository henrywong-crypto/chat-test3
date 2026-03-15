use serde::Serialize;

#[derive(Serialize)]
pub struct HelloPayload {
    #[serde(rename = "type")]
    pub type_: String,
    pub task_id: String,
}
