use serde::Serialize;

#[derive(Serialize)]
pub struct InterruptPayload {
    #[serde(rename = "type")]
    pub type_: String,
    pub task_id: String,
}
