mod hello;
mod interrupt;
mod query;
mod question_answer;
pub mod relay;

pub use hello::HelloPayload;
pub use interrupt::InterruptPayload;
pub use query::QueryPayload;
pub use question_answer::QuestionAnswerPayload;
pub use relay::{VmRelayHandle, start_vm_relay};

pub enum AgentMessage {
    Query {
        task_id: String,
        content: String,
        session_id: Option<String>,
        work_dir: Option<String>,
    },
    Hello {
        task_id: String,
    },
    QuestionAnswer {
        request_id: String,
        answers: serde_json::Value,
    },
    Interrupt {
        task_id: String,
    },
}
