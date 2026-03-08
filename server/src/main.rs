use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::{Html, Response};
use axum::routing::get;
use axum::Router;
use bytes::Bytes;
use firecracker_manager::{
    build_mmds_with_iam, create_vm, setup_host_networking, system_time_to_iso8601, ImdsCredential,
    VmConfig,
};
use futures::{SinkExt, StreamExt};
use russh::client::{self, Handle};
use russh::ChannelMsg;
use tokio::net::TcpListener;

#[derive(Clone)]
struct AppState {
    kernel_path: PathBuf,
    rootfs_path: PathBuf,
    socket_dir: PathBuf,
    ssh_key_path: PathBuf,
}

struct SshClient;

#[async_trait]
impl client::Handler for SshClient {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        _server_public_key: &russh_keys::key::PublicKey,
    ) -> Result<bool, Self::Error> {
        // Ephemeral VMs: accept any host key.
        Ok(true)
    }
}

#[tokio::main]
async fn main() {
    setup_host_networking().await;
    let state = load_app_state();
    let app = Router::new()
        .route("/", get(handle_index))
        .route("/ws", get(handle_attach_websocket))
        .with_state(state);

    let listener = TcpListener::bind("0.0.0.0:3000").await.unwrap();
    println!("listening on http://0.0.0.0:3000");
    axum::serve(listener, app).await.unwrap();
}

async fn handle_index() -> Html<&'static str> {
    Html(FRONTEND_HTML)
}

async fn handle_attach_websocket(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> Response {
    ws.on_upgrade(|socket| run_vm_session(socket, state))
}

async fn run_vm_session(ws: WebSocket, state: AppState) {
    let iam_creds = fetch_host_iam_credentials().await;
    let vm_config = build_vm_config(&state, iam_creds);
    let vm = match create_vm(&vm_config).await {
        Ok(vm) => vm,
        Err(e) => {
            eprintln!("failed to create vm: {e}");
            return;
        }
    };

    let guest_ip = vm.guest_ip.clone();
    let _guard = vm.into_guard();

    if let Err(e) = run_ssh_relay(&guest_ip, &state.ssh_key_path, ws).await {
        eprintln!("SSH session error: {e}");
    }
}

async fn run_ssh_relay(guest_ip: &str, key_path: &Path, ws: WebSocket) -> Result<()> {
    let key = Arc::new(
        russh_keys::load_secret_key(key_path, None).context("failed to load SSH private key")?,
    );
    let config = Arc::new(client::Config::default());
    let addr = format!("{guest_ip}:22");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);

    // Retry until sshd is up (VM still booting).
    let mut handle: Handle<SshClient> = loop {
        match client::connect(config.clone(), addr.as_str(), SshClient).await {
            Ok(h) => break h,
            Err(_) if tokio::time::Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            Err(e) => bail!("SSH connect timed out: {e}"),
        }
    };

    let ok = handle.authenticate_publickey("root", key).await?;
    if !ok {
        bail!("SSH authentication rejected");
    }

    let mut channel = handle.channel_open_session().await?;
    channel.request_pty(false, "xterm-256color", 80, 24, 0, 0, &[]).await?;
    channel.request_shell(false).await?;

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
                        if channel.data(&data[..]).await.is_err() {
                            break;
                        }
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

fn build_vm_config(state: &AppState, iam_creds: Option<(String, ImdsCredential)>) -> VmConfig {
    let vm_id = uuid::Uuid::new_v4().to_string();
    let (mmds_metadata, mmds_imds_compat) = match iam_creds {
        Some((role_name, cred)) => (build_mmds_with_iam(&vm_id, &role_name, &cred), true),
        None => (
            serde_json::json!({ "latest": { "meta-data": { "instance-id": &vm_id } } }),
            false,
        ),
    };
    VmConfig {
        id: vm_id,
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
            std::env::var("SSH_KEY_PATH")
                .unwrap_or_else(|_| "/var/lib/fc/id_rsa".to_string()),
        ),
    }
}

const FRONTEND_HTML: &str = r#"<!DOCTYPE html>
<html>
<head>
  <meta charset="utf-8" />
  <title>vm terminal</title>
  <link rel="stylesheet" href="https://cdn.jsdelivr.net/npm/xterm@5/css/xterm.css" />
  <style>
    html, body { margin: 0; padding: 0; background: #000; width: 100%; height: 100%; overflow: hidden; }
    #terminal { width: 100%; height: 100%; }
  </style>
</head>
<body>
  <div id="terminal"></div>
  <script src="https://cdn.jsdelivr.net/npm/xterm@5/lib/xterm.js"></script>
  <script src="https://cdn.jsdelivr.net/npm/xterm-addon-fit@0.8/lib/xterm-addon-fit.js"></script>
  <script>
    const container = document.getElementById('terminal');
    const term = new Terminal({ cursorBlink: true });
    const fitAddon = new FitAddon.FitAddon();
    term.loadAddon(fitAddon);
    term.open(container);

    const ws = new WebSocket('ws://' + location.host + '/ws');
    ws.binaryType = 'arraybuffer';

    function sendResize() {
      if (ws.readyState === WebSocket.OPEN) {
        ws.send(JSON.stringify({ type: 'resize', rows: term.rows, cols: term.cols }));
      }
    }

    term.onResize(sendResize);

    ws.onopen = () => {
      term.onData(data => ws.send(new TextEncoder().encode(data)));
      sendResize();
    };

    ws.onmessage = event => term.write(new Uint8Array(event.data));
    ws.onclose = () => term.write('\r\nconnection closed\r\n');

    new ResizeObserver(() => fitAddon.fit()).observe(container);

    document.addEventListener('keydown', e => {
      if (e.key === 'F11') {
        e.preventDefault();
        document.fullscreenElement ? document.exitFullscreen() : container.requestFullscreen();
      }
    });
  </script>
</body>
</html>"#;
