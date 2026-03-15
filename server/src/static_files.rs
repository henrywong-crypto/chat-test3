use anyhow::{Context, Result};
use axum::{
    extract::State,
    http::header,
    response::IntoResponse,
};
use bytes::Bytes;
use std::path::Path;

use crate::state::AppState;

pub(crate) struct StaticAssets {
    pub(crate) app_js: Bytes,
    pub(crate) styles_css: Bytes,
    pub(crate) app_js_version: String,
    pub(crate) styles_css_version: String,
}

pub(crate) fn load_static_assets(static_dir: &Path) -> Result<StaticAssets> {
    let app_js = std::fs::read(static_dir.join("app.js"))
        .with_context(|| format!("failed to read {}/app.js", static_dir.display()))?;
    let styles_css = std::fs::read(static_dir.join("styles.css"))
        .with_context(|| format!("failed to read {}/styles.css", static_dir.display()))?;
    let app_js_version = format!("{:x}", fnv1a(&app_js));
    let styles_css_version = format!("{:x}", fnv1a(&styles_css));
    Ok(StaticAssets {
        app_js: Bytes::from(app_js),
        styles_css: Bytes::from(styles_css),
        app_js_version,
        styles_css_version,
    })
}

pub(crate) async fn serve_app_js(State(state): State<AppState>) -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "application/javascript; charset=utf-8"),
            (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
        ],
        state.static_assets.app_js.clone(),
    )
}

pub(crate) async fn serve_styles_css(State(state): State<AppState>) -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "text/css; charset=utf-8"),
            (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
        ],
        state.static_assets.styles_css.clone(),
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
