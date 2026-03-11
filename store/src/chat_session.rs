use anyhow::Result;
use chrono::{DateTime, TimeZone, Utc};
use serde::Serialize;
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Serialize)]
pub struct ChatSession {
    pub session_id: String,
    pub title: String,
    pub last_active_at: DateTime<Utc>,
}

#[derive(sqlx::FromRow)]
struct ChatSessionRow {
    session_id: String,
    title: String,
    last_active_at: OffsetDateTime,
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
    let rows = sqlx::query_as!(
        ChatSessionRow,
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
    let chat_sessions = rows.into_iter().map(build_chat_session).collect();
    Ok(chat_sessions)
}

fn build_chat_session(row: ChatSessionRow) -> ChatSession {
    let last_active_at = Utc
        .timestamp_opt(row.last_active_at.unix_timestamp(), 0)
        .single()
        .unwrap_or_default();
    ChatSession { session_id: row.session_id, title: row.title, last_active_at }
}
