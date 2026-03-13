use serde::Deserialize;

use crate::Content;

#[derive(Deserialize)]
pub(crate) struct JournalMessage {
    pub(crate) role: String,
    pub(crate) content: Content,
}

#[derive(Deserialize)]
pub(crate) struct JournalEntry {
    #[serde(rename = "type")]
    pub(crate) entry_type: String,
    pub(crate) message: JournalMessage,
}
