use anyhow::Result;
use serde::Serialize;
use std::path::Path;

use crate::http::send_put;

#[derive(Serialize)]
pub struct MachineConfig {
    pub vcpu_count: u8,
    pub mem_size_mib: u32,
}

#[derive(Serialize)]
pub struct BootSource {
    pub kernel_image_path: String,
    pub boot_args: String,
}

pub async fn set_machine_config(socket_path: &Path, machine_config: &MachineConfig) -> Result<()> {
    let body = serde_json::to_vec(machine_config)?;
    send_put(socket_path, "/machine-config", body).await
}

pub async fn set_boot_source(socket_path: &Path, boot_source: &BootSource) -> Result<()> {
    let body = serde_json::to_vec(boot_source)?;
    send_put(socket_path, "/boot-source", body).await
}

pub async fn start_instance(socket_path: &Path) -> Result<()> {
    let body = br#"{"action_type":"InstanceStart"}"#.to_vec();
    send_put(socket_path, "/actions", body).await
}
