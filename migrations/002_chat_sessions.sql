CREATE TABLE chat_sessions (
    id             UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id        UUID        NOT NULL REFERENCES users(id),
    vm_id          TEXT        NOT NULL,
    session_id     TEXT        NOT NULL,
    title          TEXT        NOT NULL,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_active_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (user_id, session_id)
);
