use anyhow::Result;
use serde::Serialize;
use std::path::Path;

use crate::http::send_put;

#[derive(Serialize)]
pub struct MmdsConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub network_interfaces: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ipv4_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub imds_compat: Option<bool>,
}

pub async fn set_mmds_config(socket_path: &Path, config: &MmdsConfig) -> Result<()> {
    let body = serde_json::to_vec(config)?;
    send_put(socket_path, "/mmds/config", body).await
}

pub async fn put_mmds(socket_path: &Path, metadata: &serde_json::Value) -> Result<()> {
    let body = serde_json::to_vec(metadata)?;
    send_put(socket_path, "/mmds", body).await
}
