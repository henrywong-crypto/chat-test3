use serde::{Deserialize, Serialize};

mod history;
mod project;
mod session;

pub use history::{fetch_chat_history, ChatHistory, ChatMessage};
pub use session::{list_chat_sessions, ChatSession};

#[derive(Deserialize, Serialize)]
#[serde(untagged)]
pub enum Content {
    Text(String),
    Blocks(Vec<ContentBlock>),
}

#[derive(Deserialize, Serialize)]
pub struct ContentBlock {
    #[serde(rename = "type")]
    pub block_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(flatten)]
    fields: serde_json::Map<String, serde_json::Value>,
}
