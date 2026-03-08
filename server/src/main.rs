use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use bytes::Bytes;
use firecracker_manager::{
    build_mmds_with_iam, create_vm, reconcile_vms, setup_host_networking, system_time_to_iso8601,
    ImdsCredential, VmConfig, VmGuard,
};
use futures::{SinkExt, StreamExt};
use russh::client::{self, Handle};
use russh::ChannelMsg;
use serde::Serialize;
use tokio::net::TcpListener;

// ── VM registry ───────────────────────────────────────────────────────────────

struct VmEntry {
    guest_ip: String,
    created_at: u64,
    _guard: VmGuard,
}

#[derive(Serialize)]
struct VmInfo {
    id: String,
    guest_ip: String,
    created_at: u64,
}

type VmRegistry = Arc<Mutex<HashMap<String, VmEntry>>>;

// ── App state ─────────────────────────────────────────────────────────────────

#[derive(Clone)]
struct AppState {
    kernel_path: PathBuf,
    rootfs_path: PathBuf,
    socket_dir: PathBuf,
    ssh_key_path: PathBuf,
    ssh_user: String,
    vms: VmRegistry,
}

// ── SSH handler ───────────────────────────────────────────────────────────────

struct SshClient;

#[async_trait]
impl client::Handler for SshClient {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        _server_public_key: &russh_keys::key::PublicKey,
    ) -> Result<bool, Self::Error> {
        Ok(true)
    }
}

// ── Entry point ───────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    setup_host_networking().await;
    let state = load_app_state();

    for (id, guard) in reconcile_vms(&state.socket_dir).await {
        let created_at = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
        state.vms.lock().unwrap().insert(id, VmEntry {
            guest_ip: guard.guest_ip.clone(),
            created_at,
            _guard: guard,
        });
    }

    let app = Router::new()
        .route("/", get(handle_index))
        .route("/vms", get(list_vms).post(create_vm_endpoint))
        .route("/vms/{id}", get(get_vm).delete(delete_vm_endpoint))
        .route("/ws/{id}", get(handle_websocket))
        .with_state(state);

    let port = std::env::var("PORT").unwrap_or_else(|_| "3000".to_string());
    let listener = TcpListener::bind(format!("0.0.0.0:{port}"))
        .await
        .with_context(|| format!("failed to bind to 0.0.0.0:{port}"))?;
    println!("listening on http://0.0.0.0:{port}");
    axum::serve(listener, app).await.context("server error")?;
    Ok(())
}

// ── REST handlers ─────────────────────────────────────────────────────────────

async fn handle_index() -> Html<&'static str> {
    Html(FRONTEND_HTML)
}

async fn list_vms(State(state): State<AppState>) -> Json<Vec<VmInfo>> {
    let registry = state.vms.lock().unwrap();
    let mut vms: Vec<VmInfo> = registry
        .iter()
        .map(|(id, e)| VmInfo {
            id: id.clone(),
            guest_ip: e.guest_ip.clone(),
            created_at: e.created_at,
        })
        .collect();
    vms.sort_by_key(|v| v.created_at);
    Json(vms)
}

async fn get_vm(
    Path(vm_id): Path<String>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    match state.vms.lock().unwrap().get(&vm_id) {
        Some(e) => Json(VmInfo {
            id: vm_id,
            guest_ip: e.guest_ip.clone(),
            created_at: e.created_at,
        })
        .into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn create_vm_endpoint(
    State(state): State<AppState>,
) -> impl IntoResponse {
    let iam_creds = fetch_host_iam_credentials().await;
    let vm_id = uuid::Uuid::new_v4().to_string();
    let vm_config = build_vm_config(&state, &vm_id, iam_creds);

    let vm = match create_vm(&vm_config).await {
        Ok(vm) => vm,
        Err(e) => {
            eprintln!("failed to create vm: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let created_at = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    let info = VmInfo { id: vm.id.clone(), guest_ip: vm.guest_ip.clone(), created_at };

    state.vms.lock().unwrap().insert(vm.id.clone(), VmEntry {
        guest_ip: vm.guest_ip.clone(),
        created_at,
        _guard: vm.into_guard(),
    });

    (StatusCode::CREATED, Json(info)).into_response()
}

async fn delete_vm_endpoint(
    Path(vm_id): Path<String>,
    State(state): State<AppState>,
) -> StatusCode {
    match state.vms.lock().unwrap().remove(&vm_id) {
        Some(entry) => {
            entry._guard.delete();
            StatusCode::NO_CONTENT
        }
        None => StatusCode::NOT_FOUND,
    }
}

// ── WebSocket handler ─────────────────────────────────────────────────────────

async fn handle_websocket(
    Path(vm_id): Path<String>,
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> Response {
    ws.on_upgrade(|socket| run_terminal_session(socket, state, vm_id))
}

async fn run_terminal_session(ws: WebSocket, state: AppState, vm_id: String) {
    let guest_ip = match state.vms.lock().unwrap().get(&vm_id) {
        Some(e) => e.guest_ip.clone(),
        None => {
            eprintln!("VM {vm_id} not found");
            return;
        }
    };

    if let Err(e) = run_ssh_relay(&guest_ip, &state.ssh_key_path, &state.ssh_user, ws).await {
        eprintln!("SSH session error [{vm_id}]: {e}");
    }
}

// ── SSH relay ─────────────────────────────────────────────────────────────────

async fn run_ssh_relay(
    guest_ip: &str,
    key_path: &std::path::Path,
    ssh_user: &str,
    ws: WebSocket,
) -> Result<()> {
    let keypair = Arc::new(
        russh_keys::load_secret_key(key_path, None).context("failed to load SSH private key")?,
    );
    let config = Arc::new(client::Config::default());
    let addr = format!("{guest_ip}:22");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);

    let mut handle: Handle<SshClient> = loop {
        match client::connect(config.clone(), addr.as_str(), SshClient).await {
            Ok(h) => break h,
            Err(_) if tokio::time::Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            Err(e) => bail!("SSH connect timed out: {e}"),
        }
    };

    let ok = handle.authenticate_publickey(ssh_user, keypair).await?;
    if !ok {
        bail!("SSH authentication rejected");
    }

    let mut channel = handle.channel_open_session().await?;
    channel.request_pty(false, "xterm-256color", 80, 24, 0, 0, &[]).await?;
    channel.exec(false, "tmux new-session -A -s main").await?;

    let (mut ws_sender, mut ws_receiver) = ws.split();
    loop {
        tokio::select! {
            msg = channel.wait() => {
                match msg {
                    Some(ChannelMsg::Data { ref data }) => {
                        if ws_sender.send(Message::Binary(Bytes::copy_from_slice(data))).await.is_err() {
                            break;
                        }
                    }
                    Some(ChannelMsg::ExitStatus { .. }) | None => break,
                    _ => {}
                }
            }
            ws_msg = ws_receiver.next() => {
                match ws_msg {
                    Some(Ok(Message::Binary(data))) => {
                        if channel.data(&data[..]).await.is_err() { break; }
                    }
                    Some(Ok(Message::Text(text))) => {
                        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                            if json["type"] == "resize" {
                                let cols = json["cols"].as_u64().unwrap_or(80) as u32;
                                let rows = json["rows"].as_u64().unwrap_or(24) as u32;
                                let _ = channel.window_change(cols, rows, 0, 0).await;
                            }
                        }
                    }
                    _ => break,
                }
            }
        }
    }
    Ok(())
}

// ── VM config ─────────────────────────────────────────────────────────────────

fn build_vm_config(
    state: &AppState,
    vm_id: &str,
    iam_creds: Option<(String, ImdsCredential)>,
) -> VmConfig {
    let (mmds_metadata, mmds_imds_compat) = match iam_creds {
        Some((role_name, cred)) => (build_mmds_with_iam(vm_id, &role_name, &cred), true),
        None => (
            serde_json::json!({ "latest": { "meta-data": { "instance-id": vm_id } } }),
            false,
        ),
    };
    VmConfig {
        id: vm_id.to_string(),
        socket_dir: state.socket_dir.clone(),
        kernel_path: state.kernel_path.clone(),
        rootfs_path: state.rootfs_path.clone(),
        vcpu_count: 2,
        mem_size_mib: 4096,
        boot_args: "reboot=k panic=1 quiet loglevel=3 selinux=0".to_string(),
        mmds_metadata: Some(mmds_metadata),
        mmds_imds_compat,
    }
}

async fn fetch_host_iam_credentials() -> Option<(String, ImdsCredential)> {
    use aws_config::default_provider::credentials::DefaultCredentialsChain;
    use aws_credential_types::provider::ProvideCredentials;

    let provider = DefaultCredentialsChain::builder().build().await;
    let creds = provider
        .provide_credentials()
        .await
        .map_err(|e| eprintln!("failed to fetch host credentials: {e}"))
        .ok()?;
    let role_name = std::env::var("AWS_ROLE_NAME").unwrap_or_else(|_| "vm-role".to_string());
    let expiration = creds
        .expiry()
        .map(system_time_to_iso8601)
        .unwrap_or_else(|| "2099-01-01T00:00:00Z".to_string());
    Some((
        role_name,
        ImdsCredential::new(
            creds.access_key_id(),
            creds.secret_access_key(),
            creds.session_token().unwrap_or(""),
            expiration,
        ),
    ))
}

fn load_app_state() -> AppState {
    AppState {
        kernel_path: PathBuf::from(
            std::env::var("KERNEL_PATH").unwrap_or_else(|_| "/var/lib/fc/vmlinux".to_string()),
        )
        .canonicalize()
        .expect("KERNEL_PATH does not exist"),
        rootfs_path: PathBuf::from(
            std::env::var("ROOTFS_PATH")
                .unwrap_or_else(|_| "/var/lib/fc/rootfs.ext4".to_string()),
        )
        .canonicalize()
        .expect("ROOTFS_PATH does not exist"),
        socket_dir: PathBuf::from(
            std::env::var("SOCKET_DIR").unwrap_or_else(|_| "/tmp".to_string()),
        ),
        ssh_key_path: PathBuf::from(
            std::env::var("SSH_KEY_PATH").unwrap_or_else(|_| "/var/lib/fc/id_rsa".to_string()),
        ),
        ssh_user: std::env::var("SSH_USER").unwrap_or_else(|_| "root".to_string()),
        vms: Arc::new(Mutex::new(HashMap::new())),
    }
}

const FRONTEND_HTML: &str = r#"<!DOCTYPE html>
<html>
<head>
  <meta charset="utf-8" />
  <title>vm-terminal</title>
  <link rel="stylesheet" href="https://cdn.jsdelivr.net/npm/xterm@5/css/xterm.css" />
  <style>
    *, *::before, *::after { box-sizing: border-box; }
    body { margin: 0; background: #0d1117; color: #c9d1d9; font-family: ui-monospace, monospace; }
    #list-view { padding: 24px; }
    .list-header { display: flex; justify-content: space-between; align-items: center; margin-bottom: 20px; }
    h1 { margin: 0; color: #58a6ff; font-size: 20px; }
    table { width: 100%; border-collapse: collapse; }
    th { text-align: left; padding: 8px 12px; color: #8b949e; font-size: 11px;
         text-transform: uppercase; border-bottom: 1px solid #21262d; }
    td { padding: 10px 12px; border-bottom: 1px solid #161b22; font-size: 13px; }
    tr:hover td { background: #161b22; }
    .empty { color: #8b949e; padding: 32px 0; text-align: center; }
    #terminal-view { display: none; position: fixed; inset: 0; flex-direction: column; background: #000; }
    #term-header { display: flex; align-items: center; gap: 12px; padding: 6px 12px;
                   background: #161b22; border-bottom: 1px solid #30363d; flex-shrink: 0; }
    #term-vm-id { font-size: 12px; color: #8b949e; }
    #term-container { flex: 1; min-height: 0; }
    button { background: #21262d; color: #c9d1d9; border: 1px solid #30363d;
             padding: 5px 12px; cursor: pointer; border-radius: 6px; font-size: 13px; }
    button:hover { background: #30363d; }
    .btn-primary { background: #238636; border-color: #2ea043; }
    .btn-primary:hover { background: #2ea043; }
    .btn-danger { background: #6e1b1b; border-color: #da3633; }
    .btn-danger:hover { background: #da3633; }
    .btn-primary:disabled { background: #1a4a23; border-color: #1a4a23; color: #4d7a57; cursor: default; }
  </style>
</head>
<body>
<div id="list-view">
  <div class="list-header">
    <h1>vm-terminal</h1>
    <button id="new-btn" class="btn-primary" onclick="newVm()">+ New VM</button>
  </div>
  <div id="vm-table-wrap"></div>
</div>
<div id="terminal-view">
  <div id="term-header">
    <button onclick="backToList()">&#8592; VMs</button>
    <span id="term-vm-id"></span>
  </div>
  <div id="term-container"></div>
</div>
<script src="https://cdn.jsdelivr.net/npm/xterm@5/lib/xterm.js"></script>
<script src="https://cdn.jsdelivr.net/npm/xterm-addon-fit@0.8/lib/xterm-addon-fit.js"></script>
<script>
  let ws = null, term = null, fitAddon = null, refreshTimer = null;

  function ago(secs) {
    const d = Math.floor(Date.now() / 1000) - secs;
    if (d < 60) return d + 's ago';
    if (d < 3600) return Math.floor(d/60) + 'm ago';
    if (d < 86400) return Math.floor(d/3600) + 'h ago';
    return Math.floor(d/86400) + 'd ago';
  }

  async function loadVms() {
    const vms = await fetch('/vms').then(r => r.json());
    const wrap = document.getElementById('vm-table-wrap');
    if (!vms.length) { wrap.innerHTML = '<p class="empty">No running VMs.</p>'; return; }
    wrap.innerHTML = `<table><thead><tr><th>ID</th><th>IP</th><th>Started</th><th></th></tr></thead><tbody>
      ${vms.map(v => `<tr>
        <td title="${v.id}">${v.id.slice(0,8)}&hellip;</td>
        <td>${v.guest_ip}</td>
        <td>${ago(v.created_at)}</td>
        <td style="display:flex;gap:6px">
          <button onclick="connectVm('${v.id}')">Connect</button>
          <button class="btn-danger" onclick="deleteVm('${v.id}')">Delete</button>
        </td></tr>`).join('')}
      </tbody></table>`;
  }

  function startRefresh() { loadVms(); refreshTimer = setInterval(loadVms, 5000); }
  function stopRefresh() { clearInterval(refreshTimer); refreshTimer = null; }

  function backToList() {
    if (ws) { ws.close(); ws = null; }
    if (term) { term.dispose(); term = null; fitAddon = null; }
    document.getElementById('term-container').innerHTML = '';
    document.getElementById('terminal-view').style.display = 'none';
    document.getElementById('list-view').style.display = '';
    startRefresh();
  }

  function openTerminal(vmId) {
    stopRefresh();
    document.getElementById('list-view').style.display = 'none';
    const tv = document.getElementById('terminal-view');
    tv.style.display = 'flex';
    document.getElementById('term-vm-id').textContent = vmId.slice(0,8) + '\u2026';

    term = new Terminal({ cursorBlink: true });
    fitAddon = new FitAddon.FitAddon();
    term.loadAddon(fitAddon);
    term.open(document.getElementById('term-container'));

    ws = new WebSocket('ws://' + location.host + '/ws/' + vmId);
    ws.binaryType = 'arraybuffer';
    function sendResize() {
      if (ws.readyState === WebSocket.OPEN)
        ws.send(JSON.stringify({ type: 'resize', rows: term.rows, cols: term.cols }));
    }
    term.onResize(sendResize);
    ws.onopen = () => { term.onData(d => ws.send(new TextEncoder().encode(d))); sendResize(); };
    ws.onmessage = e => term.write(new Uint8Array(e.data));
    ws.onclose = () => term.write('\r\nconnection closed\r\n');
    new ResizeObserver(() => fitAddon.fit()).observe(document.getElementById('term-container'));
  }

  async function newVm() {
    const btn = document.getElementById('new-btn');
    btn.disabled = true; btn.textContent = 'Starting\u2026';
    try {
      const res = await fetch('/vms', { method: 'POST' });
      if (!res.ok) { alert('Failed to create VM'); return; }
      const vm = await res.json();
      openTerminal(vm.id);
    } finally { btn.disabled = false; btn.textContent = '+ New VM'; }
  }

  function connectVm(id) { openTerminal(id); }

  async function deleteVm(id) {
    await fetch('/vms/' + id, { method: 'DELETE' });
    loadVms();
  }

  startRefresh();
</script>
</body>
</html>"#;
