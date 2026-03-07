use std::path::PathBuf;
use std::os::fd::{AsRawFd, RawFd};

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::{Html, Response};
use axum::routing::get;
use axum::Router;
use bytes::Bytes;
use firecracker_manager::{build_mmds_with_iam, create_vm, setup_host_networking, system_time_to_iso8601, ImdsCredential, VmConfig};
use futures::{SinkExt, StreamExt};
use terminal_bridge::{resize_pty_fd, PtyMaster};
use tokio::io::{split, AsyncReadExt, AsyncWriteExt, ReadHalf, WriteHalf};
use tokio::net::TcpListener;

#[derive(Clone)]
struct AppState {
    kernel_path: PathBuf,
    rootfs_path: PathBuf,
    socket_dir: PathBuf,
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

    let (pty_master, _guard) = vm.into_pty_master();
    let pty_raw_fd = pty_master.as_raw_fd();
    let _ = resize_pty_fd(pty_raw_fd, 24, 80);
    let (pty_reader, pty_writer) = split(pty_master);
    let (ws_sender, ws_receiver) = ws.split();

    tokio::select! {
        _ = relay_pty_to_websocket(pty_reader, ws_sender) => {}
        _ = relay_websocket_to_pty(ws_receiver, pty_writer, pty_raw_fd) => {}
    }
}

async fn relay_pty_to_websocket(
    mut pty_reader: ReadHalf<PtyMaster>,
    mut ws_sender: futures::stream::SplitSink<WebSocket, Message>,
) {
    let mut buf = vec![0u8; 4096];
    loop {
        match pty_reader.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let msg = Message::Binary(Bytes::copy_from_slice(&buf[..n]));
                if ws_sender.send(msg).await.is_err() {
                    break;
                }
            }
        }
    }
}

async fn relay_websocket_to_pty(
    mut ws_receiver: futures::stream::SplitStream<WebSocket>,
    mut pty_writer: WriteHalf<PtyMaster>,
    pty_raw_fd: RawFd,
) {
    use std::time::Duration;
    use tokio::time::Instant;

    let mut last_rows: u16 = 24;
    let mut last_cols: u16 = 80;
    let mut stty_sent = false;
    let mut retry_at: Option<Instant> = None;
    const RETRY_DELAY: Duration = Duration::from_millis(400);

    loop {
        tokio::select! {
            opt = ws_receiver.next() => {
                let msg = match opt {
                    Some(Ok(m)) => m,
                    _ => break,
                };
                match msg {
                    Message::Binary(data) => {
                        if pty_writer.write_all(&data).await.is_err() {
                            break;
                        }
                    }
                    Message::Text(text) => {
                        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                            if json.get("type").and_then(|v| v.as_str()) == Some("resize") {
                                let rows = json["rows"].as_u64().unwrap_or(24) as u16;
                                let cols = json["cols"].as_u64().unwrap_or(80) as u16;
                                last_rows = rows;
                                last_cols = cols;
                                let _ = resize_pty_fd(pty_raw_fd, rows, cols);
                                // Guest serial (ttyS0) doesn't get TIOCSWINSZ from host; only
                                // stty inside the guest sets winsize. So TUI apps (less, vim)
                                // need stty to see full terminal size. Send once on first resize,
                                // and again after a short delay in case the first size was wrong.
                                if !stty_sent {
                                    stty_sent = true;
                                    let cmd = format!("stty cols {} rows {}; export TERM=xterm-256color\n", cols, rows);
                                    let _ = pty_writer.write_all(cmd.as_bytes()).await;
                                    retry_at = Some(Instant::now() + RETRY_DELAY);
                                }
                            }
                        }
                    }
                    Message::Close(_) => break,
                    _ => {}
                }
            }
            _ = tokio::time::sleep_until(
                retry_at.as_ref().copied().unwrap_or(Instant::now() + Duration::from_secs(999999))
            ), if retry_at.is_some() => {
                retry_at = None;
                let cmd = format!("stty cols {} rows {}; export TERM=xterm-256color\n", last_cols, last_rows);
                let _ = pty_writer.write_all(cmd.as_bytes()).await;
            }
        }
    }
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
        boot_args: "console=ttyS0 reboot=k panic=1 quiet loglevel=3 selinux=0".to_string(),
        mmds_metadata: Some(mmds_metadata),
        mmds_imds_compat,
    }
}

async fn fetch_host_iam_credentials() -> Option<(String, ImdsCredential)> {
    use aws_config::default_provider::credentials::DefaultCredentialsChain;
    use aws_credential_types::provider::ProvideCredentials;

    let provider = DefaultCredentialsChain::builder().build().await;
    let creds = provider.provide_credentials().await
        .map_err(|e| eprintln!("failed to fetch host credentials: {e}"))
        .ok()?;
    let role_name =
        std::env::var("AWS_ROLE_NAME").unwrap_or_else(|_| "vm-role".to_string());
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
    }
}

const FRONTEND_HTML: &str = r#"<!DOCTYPE html>
<html>
<head>
  <meta charset="utf-8" />
  <title>vm terminal</title>
  <link rel="stylesheet" href="https://cdn.jsdelivr.net/npm/xterm@5/css/xterm.css" />
  <style>
    html, body { margin: 0; padding: 0; background: #000; height: 100%; overflow: hidden; }
    #terminal {
      position: fixed;
      top: 0; left: 0; right: 0; bottom: 0;
      width: 100%; height: 100%;
    }
    #terminal .xterm { width: 100% !important; height: 100% !important; }
    #terminal .xterm-viewport { overflow: hidden !important; }
  </style>
</head>
<body>
  <div id="terminal"></div>
  <script src="https://cdn.jsdelivr.net/npm/xterm@5/lib/xterm.js"></script>
  <script src="https://cdn.jsdelivr.net/npm/xterm-addon-fit@0.8/lib/xterm-addon-fit.js"></script>
  <script>
    const term = new Terminal({ cursorBlink: true });
    const fitAddon = new FitAddon.FitAddon();
    term.loadAddon(fitAddon);
    term.open(document.getElementById('terminal'));
    // Defer fit until container has layout dimensions (fixes very small initial size)
    requestAnimationFrame(() => {
      requestAnimationFrame(() => {
        fitAddon.fit();
      });
    });

    function doFit() {
      requestAnimationFrame(() => { fitAddon.fit(); });
    }

    const ws = new WebSocket('ws://' + location.host + '/ws');
    ws.binaryType = 'arraybuffer';

    term.onResize(({ rows, cols }) => {
      if (ws.readyState === WebSocket.OPEN) {
        ws.send(JSON.stringify({ type: 'resize', rows, cols }));
      }
    });

    ws.onopen = () => {
      term.onData(data => ws.send(new TextEncoder().encode(data)));
      doFit();
    };

    ws.onmessage = event => term.write(new Uint8Array(event.data));

    ws.onclose = () => term.write('\r\nconnection closed\r\n');

    window.addEventListener('resize', () => doFit());
    const resizeObs = new ResizeObserver(() => doFit());
    resizeObs.observe(document.getElementById('terminal'));

    // Fullscreen: F11 toggles; must call fit() after change so terminal resizes (xterm#1053)
    function toggleFullscreen() {
      const el = document.getElementById('terminal');
      const doc = document;
      const req = el.requestFullscreen || el.webkitRequestFullscreen || el.mozRequestFullScreen || el.msRequestFullscreen;
      const exit = doc.exitFullscreen || doc.webkitExitFullscreen || doc.mozCancelFullScreen || doc.msExitFullscreen;
      const isFull = doc.fullscreenElement || doc.webkitFullscreenElement || doc.mozFullScreenElement || doc.msFullscreenElement;
      if (isFull) exit.call(doc);
      else req.call(el);
    }
    document.addEventListener('keydown', (e) => {
      if (e.key === 'F11') { e.preventDefault(); toggleFullscreen(); }
    });
    document.addEventListener('fullscreenchange', () => doFit());
    document.addEventListener('webkitfullscreenchange', () => doFit());
  </script>
</body>
</html>"#;
