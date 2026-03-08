use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
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
use serde::{Deserialize, Serialize};
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

#[derive(Deserialize)]
struct CreateVmRequest {
    socket_path: String,
}

type VmRegistry = Arc<Mutex<HashMap<String, VmEntry>>>;

// ── App state ─────────────────────────────────────────────────────────────────

#[derive(Clone)]
struct AppState {
    kernel_path: PathBuf,
    rootfs_path: PathBuf,
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

    let socket_dir = PathBuf::from(
        std::env::var("SOCKET_DIR").unwrap_or_else(|_| "/tmp".to_string()),
    );
    for (id, guard) in reconcile_vms(&socket_dir).await {
        let created_at = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
        state.vms.lock().unwrap().insert(id, VmEntry {
            guest_ip: guard.guest_ip.clone(),
            created_at,
            _guard: guard,
        });
    }

    let app = Router::new()
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
        ssh_key_path: PathBuf::from(
            std::env::var("SSH_KEY_PATH").unwrap_or_else(|_| "/var/lib/fc/id_rsa".to_string()),
        ),
        ssh_user: std::env::var("SSH_USER").unwrap_or_else(|_| "root".to_string()),
        vms: Arc::new(Mutex::new(HashMap::new())),
    }
}
