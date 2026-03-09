mod auth;
mod handlers;
mod ssh;
mod state;
mod templates;
mod terminal;
mod upload;
mod vm;

use anyhow::{Context, Result};
use axum::{
    extract::Request,
    http::HeaderValue,
    middleware::{self, Next},
    response::Response,
    routing::{get, post},
    Router,
};
use firecracker_manager::{cleanup_stale_vms, setup_host_networking};
use time::Duration;
use tokio::{net::TcpListener, signal};
use tower_sessions::{cookie::SameSite, Expiry, MemoryStore, SessionManagerLayer};
use tracing::info;

use crate::{
    auth::{
        get_callback_handler, get_cognito_login_handler, get_demo_handler, get_login_handler,
        get_logout_handler,
    },
    handlers::{
        create_vm_handler, delete_user_rootfs_handler, delete_vm_handler, get_redirect_to_vms,
        get_terminal_page, get_vms_page,
    },
    state::{load_config, AppState},
    terminal::handle_ws_upgrade,
    upload::upload_file_handler,
    vm::save_all_vm_rootfs,
};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    let app_state = AppState::new(load_config()?);
    let port = app_state.port;
    cleanup_stale_vms(&app_state.socket_dir);
    setup_host_networking().await;
    let router = build_router(app_state.clone());
    serve_router(router, port, app_state).await
}

fn build_router(app_state: AppState) -> Router {
    let session_layer = build_session_layer();
    Router::new()
        .route("/", get(get_redirect_to_vms))
        .route("/vms", get(get_vms_page).post(create_vm_handler))
        .route("/vms/{id}/delete", post(delete_vm_handler))
        .route("/vms/{id}/upload", post(upload_file_handler))
        .route("/rootfs/delete", post(delete_user_rootfs_handler))
        .route("/terminal/{id}", get(get_terminal_page))
        .route("/ws/{id}", get(handle_ws_upgrade))
        .route("/login", get(get_login_handler))
        .route("/login/cognito", get(get_cognito_login_handler))
        .route("/logout", get(get_logout_handler))
        .route("/demo", get(get_demo_handler))
        .route("/callback", get(get_callback_handler))
        .with_state(app_state)
        .layer(session_layer)
        .layer(middleware::from_fn(add_security_headers))
}

fn build_session_layer() -> SessionManagerLayer<MemoryStore> {
    SessionManagerLayer::new(MemoryStore::default())
        .with_secure(true)
        .with_same_site(SameSite::Lax)
        .with_expiry(Expiry::OnInactivity(Duration::seconds(86400)))
}

async fn add_security_headers(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert("x-frame-options", HeaderValue::from_static("DENY"));
    headers.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        "referrer-policy",
        HeaderValue::from_static("strict-origin-when-cross-origin"),
    );
    headers.insert(
        "content-security-policy",
        HeaderValue::from_static(
            "default-src 'self'; \
             script-src 'self' 'unsafe-inline' https://cdn.jsdelivr.net https://cdn.tailwindcss.com; \
             style-src 'self' 'unsafe-inline' https://cdn.jsdelivr.net; \
             connect-src 'self'; \
             img-src 'self' data:; \
             font-src 'self' data:"
        ),
    );
    response
}

async fn serve_router(router: Router, port: u16, app_state: AppState) -> Result<()> {
    let tcp_listener = TcpListener::bind(format!("0.0.0.0:{port}"))
        .await
        .with_context(|| format!("failed to bind to port {port}"))?;
    info!("listening on http://0.0.0.0:{port}");
    axum::serve(tcp_listener, router)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("server error")?;
    save_all_vm_rootfs(&app_state).await;
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };
    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
    }
    info!("shutdown signal received, saving vm rootfs before exit");
}
