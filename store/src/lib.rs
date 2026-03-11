mod chat_session;
mod user;

use anyhow::Result;
pub use chat_session::{list_chat_sessions, upsert_chat_session, ChatSession};
pub use sqlx::PgPool;
pub use user::{upsert_user, User};

pub async fn connect_db(url: &str) -> Result<PgPool> {
    let pg_pool = PgPool::connect(url).await?;
    Ok(pg_pool)
}

pub async fn run_migrations(pg_pool: &PgPool) -> Result<()> {
    sqlx::migrate!("../migrations").run(pg_pool).await?;
    Ok(())
}
