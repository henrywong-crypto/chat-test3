mod frontend;
mod handlers;
mod ssh;
mod state;
mod terminal;
mod upload;
mod vm;

use anyhow::{Context, Result};
use axum::{
    routing::{delete, get, post},
    Router,
};
use clap::Parser;
use firecracker_manager::setup_host_networking;
use tokio::net::TcpListener;

use crate::{
    handlers::{create_vm_handler, delete_vm_handler, get_index, list_vms},
    state::{build_app_state, AppState, Args},
    terminal::handle_ws_upgrade,
    upload::upload_file_handler,
};

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let port = args.port;
    setup_host_networking().await;
    let app_state = build_app_state(args)?;
    let router = build_router(app_state);
    serve_router(router, port).await
}

fn build_router(app_state: AppState) -> Router {
    Router::new()
        .route("/", get(get_index))
        .route("/vms", get(list_vms).post(create_vm_handler))
        .route("/vms/{id}", delete(delete_vm_handler))
        .route("/vms/{id}/upload", post(upload_file_handler))
        .route("/ws/{id}", get(handle_ws_upgrade))
        .with_state(app_state)
}

async fn serve_router(router: Router, port: u16) -> Result<()> {
    let tcp_listener = TcpListener::bind(format!("0.0.0.0:{port}"))
        .await
        .with_context(|| format!("failed to bind to port {port}"))?;
    println!("listening on http://0.0.0.0:{port}");
    axum::serve(tcp_listener, router).await.context("server error")
}
