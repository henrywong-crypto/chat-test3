# store

- PostgreSQL backend (sqlx)
- `users`: id (UUID), cognito_sub, created_at, updated_at
- `sessions`: id (UUID), user_id (UUID), vm_socket_path, vm_pid, pty_path, status, created_at, updated_at

## Functions

```
create_user(cognito_sub: &str) -> Result<User>
get_user_by_cognito_sub(cognito_sub: &str) -> Result<Option<User>>

create_session(user_id: Uuid, vm_socket_path: &Path, vm_pid: u32, pty_path: &Path) -> Result<Session>
get_session(session_id: Uuid) -> Result<Option<Session>>
list_user_sessions(user_id: Uuid) -> Result<Vec<Session>>
delete_session(session_id: Uuid) -> Result<()>
update_session_status(session_id: Uuid, status: &SessionStatus) -> Result<()>
```
