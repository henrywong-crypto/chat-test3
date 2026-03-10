use axum::{http::header, response::IntoResponse};

const TERMINAL_JS: &[u8] = include_bytes!("../frontend/dist/terminal.js");
const FILE_MANAGER_JS: &[u8] = include_bytes!("../frontend/dist/file_manager.js");
const STYLES_CSS: &[u8] = include_bytes!("../frontend/dist/styles.css");

pub(crate) async fn serve_terminal_js() -> impl IntoResponse {
    (
        [(
            header::CONTENT_TYPE,
            "application/javascript; charset=utf-8",
        )],
        TERMINAL_JS,
    )
}

pub(crate) async fn serve_file_manager_js() -> impl IntoResponse {
    (
        [(
            header::CONTENT_TYPE,
            "application/javascript; charset=utf-8",
        )],
        FILE_MANAGER_JS,
    )
}

pub(crate) async fn serve_styles_css() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        STYLES_CSS,
    )
}
