mod hello;
mod interrupt;
mod query;
mod question_answer;

pub use hello::build_hello_payload;
pub use interrupt::build_interrupt_payload;
pub use query::build_query_payload;
pub use question_answer::build_question_answer_payload;

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
