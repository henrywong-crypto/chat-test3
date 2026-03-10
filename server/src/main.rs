mod auth;
mod download;
mod files;
mod handlers;
mod ssh;
mod state;
mod static_files;
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
use tokio::{net::TcpListener, signal, task::AbortHandle};
use tower_sessions::{cookie::SameSite, ExpiredDeletion, Expiry, SessionManagerLayer};
use tower_sessions_sqlx_store::PostgresStore;
use tracing::info;

use crate::{
    auth::{
        get_callback_handler, get_cognito_login_handler, get_login_handler, get_logout_handler,
    },
    download::download_file_handler,
    files::list_files_handler,
    handlers::{delete_user_rootfs_handler, get_or_create_terminal, get_terminal_page},
    state::{load_config, AppState},
    static_files::{serve_file_manager_js, serve_styles_css, serve_terminal_js},
    terminal::handle_ws_upgrade,
    upload::upload_file_handler,
    vm::{refresh_all_vm_mmds, save_all_vm_rootfs},
};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    let app_config = load_config()?;
    let pg_pool = store::connect_db(&app_config.database_url).await?;
    store::run_migrations(&pg_pool).await?;
    let session_store = PostgresStore::new(pg_pool.clone());
    session_store.migrate().await?;
    let deletion_task = tokio::task::spawn(
        session_store
            .clone()
            .continuously_delete_expired(tokio::time::Duration::from_secs(3600)),
    );
    let app_state = AppState::new(app_config, pg_pool);
    let port = app_state.port;
    cleanup_stale_vms(
        &app_state.socket_dir,
        &app_state.net_helper_path,
        app_state
            .use_jailer
            .then_some(&app_state.jailer_chroot_base),
    );
    setup_host_networking(&app_state.net_helper_path).await;
    let mmds_refresh_task = spawn_mmds_refresh_task(app_state.clone());
    let router = build_router(app_state.clone(), session_store);
    serve_router(
        router,
        port,
        app_state,
        deletion_task.abort_handle(),
        mmds_refresh_task.abort_handle(),
    )
    .await?;
    deletion_task.await??;
    Ok(())
}

fn build_router(app_state: AppState, session_store: PostgresStore) -> Router {
    let session_layer = build_session_layer(session_store);
    Router::new()
        .route("/", get(get_or_create_terminal))
        .route("/sessions/{id}/download", get(download_file_handler))
        .route("/sessions/{id}/ls", get(list_files_handler))
        .route("/sessions/{id}/upload", post(upload_file_handler))
        .route("/rootfs/delete", post(delete_user_rootfs_handler))
        .route("/terminal/{id}", get(get_terminal_page))
        .route("/ws/{id}", get(handle_ws_upgrade))
        .route("/login", get(get_login_handler))
        .route("/login/cognito", get(get_cognito_login_handler))
        .route("/logout", get(get_logout_handler))
        .route("/callback", get(get_callback_handler))
        .route("/static/terminal.js", get(serve_terminal_js))
        .route("/static/file-manager.js", get(serve_file_manager_js))
        .route("/static/styles.css", get(serve_styles_css))
        .with_state(app_state)
        .layer(session_layer)
        .layer(middleware::from_fn(add_security_headers))
}

fn build_session_layer(session_store: PostgresStore) -> SessionManagerLayer<PostgresStore> {
    SessionManagerLayer::new(session_store)
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
             script-src 'self'; \
             style-src 'self' 'unsafe-inline'; \
             connect-src 'self'; \
             img-src 'self' data:; \
             font-src 'self' data:",
        ),
    );
    response
}

async fn serve_router(
    router: Router,
    port: u16,
    app_state: AppState,
    deletion_task_abort_handle: AbortHandle,
    mmds_refresh_abort_handle: AbortHandle,
) -> Result<()> {
    let tcp_listener = TcpListener::bind(format!("0.0.0.0:{port}"))
        .await
        .with_context(|| format!("failed to bind to port {port}"))?;
    info!("listening on http://0.0.0.0:{port}");
    axum::serve(tcp_listener, router)
        .with_graceful_shutdown(shutdown_signal(
            deletion_task_abort_handle,
            mmds_refresh_abort_handle,
        ))
        .await
        .context("server error")?;
    save_all_vm_rootfs(&app_state).await;
    Ok(())
}

fn spawn_mmds_refresh_task(app_state: AppState) -> tokio::task::JoinHandle<()> {
    tokio::task::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(900));
        interval.tick().await;
        loop {
            interval.tick().await;
            refresh_all_vm_mmds(&app_state).await;
        }
    })
}

async fn shutdown_signal(
    deletion_task_abort_handle: AbortHandle,
    mmds_refresh_abort_handle: AbortHandle,
) {
    let ctrl_c = async {
        if let Err(e) = signal::ctrl_c().await {
            tracing::error!("failed to install Ctrl+C handler: {e}");
        }
    };
    let terminate = async {
        match signal::unix::signal(signal::unix::SignalKind::terminate()) {
            Ok(mut sig) => {
                sig.recv().await;
            }
            Err(e) => tracing::error!("failed to install SIGTERM handler: {e}"),
        }
    };
    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
    }
    info!("shutdown signal received, saving vm rootfs before exit");
    deletion_task_abort_handle.abort();
    mmds_refresh_abort_handle.abort();
}
