use axum::{http::header, response::IntoResponse};
use std::sync::OnceLock;

const APP_JS: &[u8] = include_bytes!("../../frontend/dist/app.js");
const STYLES_CSS: &[u8] = include_bytes!("../../frontend/dist/styles.css");

static APP_JS_HASH: OnceLock<String> = OnceLock::new();
static STYLES_CSS_HASH: OnceLock<String> = OnceLock::new();

pub(crate) fn app_js_version() -> &'static str {
    APP_JS_HASH.get_or_init(|| format!("{:x}", fnv1a(APP_JS)))
}

pub(crate) fn styles_css_version() -> &'static str {
    STYLES_CSS_HASH.get_or_init(|| format!("{:x}", fnv1a(STYLES_CSS)))
}

pub(crate) async fn serve_app_js() -> impl IntoResponse {
    (
        [
            (
                header::CONTENT_TYPE,
                "application/javascript; charset=utf-8",
            ),
            (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
        ],
        APP_JS,
    )
}

pub(crate) async fn serve_styles_css() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "text/css; charset=utf-8"),
            (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
        ],
        STYLES_CSS,
    )
}

fn fnv1a(data: &[u8]) -> u64 {
    let mut hash: u64 = 14695981039346656037;
    for byte in data {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(1099511628211);
    }
    hash
}
