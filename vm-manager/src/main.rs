use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse};
use axum::routing::get;
use axum::{Json, Router};
use firecracker_manager::{
    build_mmds_with_iam, create_vm, reconcile_vms, setup_host_networking, system_time_to_iso8601,
    ImdsCredential, VmConfig, VmGuard,
};
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;

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

#[derive(Deserialize)]
struct CreateVmRequest {
    socket_path: String,
}

type VmRegistry = Arc<Mutex<HashMap<String, VmEntry>>>;

#[derive(Clone)]
struct AppState {
    kernel_path: PathBuf,
    rootfs_path: PathBuf,
    server_ws_url: String,
    vms: VmRegistry,
}

#[tokio::main]
async fn main() -> Result<()> {
    setup_host_networking().await;
    let state = load_app_state();

    let socket_dir = PathBuf::from(
        std::env::var("SOCKET_DIR").unwrap_or_else(|_| "/tmp".to_string()),
    );
    let recovered = reconcile_vms(&socket_dir).await;
    if !recovered.is_empty() {
        println!("reconciled {} existing session(s)", recovered.len());
        let mut registry = state.vms.lock().unwrap();
        for (id, guard) in recovered {
            let created_at = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            registry.insert(id.clone(), VmEntry {
                guest_ip: guard.guest_ip.clone(),
                created_at,
                _guard: guard,
            });
        }
    }

    let app = Router::new()
        .route("/", get(handle_index))
        .route("/vms", get(list_vms).post(create_vm_endpoint))
        .route("/vms/{id}", get(get_vm).delete(delete_vm_endpoint))
        .with_state(state);

    let port = std::env::var("PORT").unwrap_or_else(|_| "3001".to_string());
    let listener = TcpListener::bind(format!("0.0.0.0:{port}"))
        .await
        .with_context(|| format!("failed to bind to 0.0.0.0:{port}"))?;
    println!("listening on http://0.0.0.0:{port}");
    axum::serve(listener, app).await.context("server error")?;
    Ok(())
}

async fn handle_index(State(state): State<AppState>) -> Html<String> {
    Html(FRONTEND_HTML.replace("__WS_ORIGIN__", &state.server_ws_url))
}

async fn list_vms(State(state): State<AppState>) -> Json<Vec<VmInfo>> {
    let registry = state.vms.lock().unwrap();
    let mut vms: Vec<VmInfo> = registry
        .iter()
        .map(|(id, e)| VmInfo { id: id.clone(), guest_ip: e.guest_ip.clone(), created_at: e.created_at })
        .collect();
    vms.sort_by_key(|v| v.created_at);
    Json(vms)
}

async fn get_vm(Path(vm_id): Path<String>, State(state): State<AppState>) -> impl IntoResponse {
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
    Json(req): Json<CreateVmRequest>,
) -> impl IntoResponse {
    let iam_creds = fetch_host_iam_credentials().await;
    let vm_id = uuid::Uuid::new_v4().to_string();
    let vm_config = build_vm_config(&state, &vm_id, PathBuf::from(req.socket_path), iam_creds);

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

fn build_vm_config(
    state: &AppState,
    vm_id: &str,
    socket_path: PathBuf,
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
        socket_path,
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
        server_ws_url: std::env::var("SERVER_WS_URL")
            .unwrap_or_else(|_| "ws://localhost:3000".to_string()),
        vms: Arc::new(Mutex::new(HashMap::new())),
    }
}

const FRONTEND_HTML: &str = r#"<!DOCTYPE html>
<html>
<head>
  <meta charset="utf-8" />
  <title>WebCode</title>
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
    <h1>WebCode</h1>
    <button id="new-btn" class="btn-primary" onclick="newVm()">+ New Session</button>
  </div>
  <div id="vm-table-wrap"></div>
</div>
<div id="terminal-view">
  <div id="term-header">
    <button onclick="backToList()">&#8592; Sessions</button>
    <span id="term-vm-id"></span>
  </div>
  <div id="term-container"></div>
</div>
<script src="https://cdn.jsdelivr.net/npm/xterm@5/lib/xterm.js"></script>
<script src="https://cdn.jsdelivr.net/npm/xterm-addon-fit@0.8/lib/xterm-addon-fit.js"></script>
<script>
  const WS_ORIGIN = '__WS_ORIGIN__';
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
    if (!vms.length) { wrap.innerHTML = '<p class="empty">No running sessions.</p>'; return; }
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

    ws = new WebSocket(WS_ORIGIN + '/ws/' + vmId);
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
    const socketPath = prompt('Firecracker socket path:');
    if (!socketPath) return;
    const btn = document.getElementById('new-btn');
    btn.disabled = true; btn.textContent = 'Starting\u2026';
    try {
      const res = await fetch('/vms', { method: 'POST', headers: {'Content-Type':'application/json'},
        body: JSON.stringify({ socket_path: socketPath }) });
      if (!res.ok) { alert('Failed to create session'); return; }
      const vm = await res.json();
      openTerminal(vm.id);
    } finally { btn.disabled = false; btn.textContent = '+ New Session'; }
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
