mod auth;
mod handlers;
mod ssh;
mod state;
mod templates;
mod terminal;
mod vm;

use anyhow::{Context, Result};
use axum::{
    routing::{get, post},
    Router,
};
use clap::Parser;
use firecracker_manager::{cleanup_stale_vms, setup_host_networking};
use time::Duration;
use tokio::net::TcpListener;
use tower_sessions::{cookie::SameSite, Expiry, MemoryStore, SessionManagerLayer};

use crate::{
    auth::{get_callback_handler, get_cognito_login_handler, get_demo_handler, get_login_handler, get_logout_handler},
    handlers::{create_vm_handler, delete_vm_handler, get_redirect_to_vms, get_terminal_page, get_vms_page},
    state::{build_app_state, AppState, Args},
    terminal::handle_ws_upgrade,
};

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let port = args.port;
    cleanup_stale_vms(&args.socket_dir);
    setup_host_networking().await;
    let app_state = build_app_state(args)?;
    let router = build_router(app_state);
    serve_router(router, port).await
}

fn build_router(app_state: AppState) -> Router {
    let session_layer = build_session_layer();
    Router::new()
        .route("/", get(get_redirect_to_vms))
        .route("/vms", get(get_vms_page).post(create_vm_handler))
        .route("/vms/{id}/delete", post(delete_vm_handler))
        .route("/terminal/{id}", get(get_terminal_page))
        .route("/ws/{id}", get(handle_ws_upgrade))
        .route("/login", get(get_login_handler))
        .route("/login/cognito", get(get_cognito_login_handler))
        .route("/logout", get(get_logout_handler))
        .route("/demo", get(get_demo_handler))
        .route("/callback", get(get_callback_handler))
        .with_state(app_state)
        .layer(session_layer)
}

fn build_session_layer() -> SessionManagerLayer<MemoryStore> {
    SessionManagerLayer::new(MemoryStore::default())
        .with_secure(false)
        .with_same_site(SameSite::Lax)
        .with_expiry(Expiry::OnInactivity(Duration::seconds(86400)))
}

async fn serve_router(router: Router, port: u16) -> Result<()> {
    let tcp_listener = TcpListener::bind(format!("0.0.0.0:{port}"))
        .await
        .with_context(|| format!("failed to bind to port {port}"))?;
    println!("listening on http://0.0.0.0:{port}");
    axum::serve(tcp_listener, router).await.context("server error")
}
