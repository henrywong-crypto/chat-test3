# server

- axum HTTP + WebSocket server
- Cognito JWT middleware: validates token against Cognito JWKS, extracts `cognito_sub`
- On first authenticated request: `get_or_create_user` by `cognito_sub`
- Serves static xterm.js frontend at `GET /`
- On startup: calls `reconcile_vms`

## Routes

```
POST   /sessions
GET    /sessions
DELETE /sessions/:id
GET    /ws/:session_id
GET    /
```

## Functions

```
handle_create_session(...) -> Result<Json<Session>>
handle_list_sessions(...) -> Result<Json<Vec<Session>>>
handle_delete_session(...) -> Result<()>
handle_attach_websocket(...) -> Response

relay_pty_to_websocket(pty_master: PtyMaster, ws_sender: WsSender) -> Result<()>
relay_websocket_to_pty(ws_receiver: WsReceiver, pty_master: PtyMaster) -> Result<()>
```
