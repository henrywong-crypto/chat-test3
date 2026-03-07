use std::path::PathBuf;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::{Html, Response};
use axum::routing::get;
use axum::Router;
use bytes::Bytes;
use firecracker_manager::{create_vm, setup_host_networking, VmConfig};
use futures::{SinkExt, StreamExt};
use terminal_bridge::PtyMaster;
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
    let vm_config = build_vm_config(&state);
    let vm = match create_vm(&vm_config).await {
        Ok(vm) => vm,
        Err(e) => {
            eprintln!("failed to create vm: {e}");
            return;
        }
    };

    let (pty_master, _guard) = vm.into_pty_master();
    let (pty_reader, pty_writer) = split(pty_master);
    let (ws_sender, ws_receiver) = ws.split();

    tokio::select! {
        _ = relay_pty_to_websocket(pty_reader, ws_sender) => {}
        _ = relay_websocket_to_pty(ws_receiver, pty_writer) => {}
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
) {
    while let Some(Ok(msg)) = ws_receiver.next().await {
        match msg {
            Message::Binary(data) => {
                if pty_writer.write_all(&data).await.is_err() {
                    break;
                }
            }
            Message::Text(text) => {
                if pty_writer.write_all(text.as_bytes()).await.is_err() {
                    break;
                }
            }
            Message::Close(_) => break,
            _ => {}
        }
    }
}

fn build_vm_config(state: &AppState) -> VmConfig {
    VmConfig {
        id: uuid::Uuid::new_v4().to_string(),
        socket_dir: state.socket_dir.clone(),
        kernel_path: state.kernel_path.clone(),
        rootfs_path: state.rootfs_path.clone(),
        vcpu_count: 2,
        mem_size_mib: 2048,
        boot_args: "console=ttyS0 reboot=k panic=1 root=/dev/vda".to_string(),
    }
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
    html, body { margin: 0; padding: 0; background: #000; height: 100%; }
    #terminal { height: 100%; }
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
    fitAddon.fit();

    const ws = new WebSocket('ws://' + location.host + '/ws');
    ws.binaryType = 'arraybuffer';

    ws.onopen = () => {
      term.onData(data => ws.send(new TextEncoder().encode(data)));
    };

    ws.onmessage = event => term.write(new Uint8Array(event.data));

    ws.onclose = () => term.write('\r\nconnection closed\r\n');

    window.addEventListener('resize', () => fitAddon.fit());
  </script>
</body>
</html>"#;
