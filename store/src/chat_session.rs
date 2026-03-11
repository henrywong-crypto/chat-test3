use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Serialize)]
pub struct ChatSession {
    pub session_id: String,
    pub title: String,
    pub last_active_at: DateTime<Utc>,
}

pub async fn upsert_chat_session(
    pool: &PgPool,
    user_id: Uuid,
    vm_id: &str,
    session_id: &str,
    title: &str,
) -> Result<()> {
    sqlx::query!(
        r#"
        INSERT INTO chat_sessions (user_id, vm_id, session_id, title)
        VALUES ($1, $2, $3, $4)
        ON CONFLICT (user_id, session_id)
        DO UPDATE SET title = EXCLUDED.title, last_active_at = NOW()
        "#,
        user_id,
        vm_id,
        session_id,
        title,
    )
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn list_chat_sessions(
    pool: &PgPool,
    user_id: Uuid,
    vm_id: &str,
) -> Result<Vec<ChatSession>> {
    let chat_sessions = sqlx::query_as!(
        ChatSession,
        r#"
        SELECT session_id, title, last_active_at
        FROM chat_sessions
        WHERE user_id = $1 AND vm_id = $2
        ORDER BY last_active_at DESC
        LIMIT 20
        "#,
        user_id,
        vm_id,
    )
    .fetch_all(pool)
    .await?;
    Ok(chat_sessions)
}
