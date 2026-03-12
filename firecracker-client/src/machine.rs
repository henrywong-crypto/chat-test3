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
    send_put(socket_path, "/machine-config", machine_config).await
}

pub async fn set_boot_source(socket_path: &Path, boot_source: &BootSource) -> Result<()> {
    send_put(socket_path, "/boot-source", boot_source).await
}

pub async fn start_instance(socket_path: &Path) -> Result<()> {
    send_put(
        socket_path,
        "/actions",
        &serde_json::json!({"action_type": "InstanceStart"}),
    )
    .await
}

pub async fn stop_instance(socket_path: &Path) -> Result<()> {
    send_put(
        socket_path,
        "/actions",
        &serde_json::json!({"action_type": "SendCtrlAltDel"}),
    )
    .await
}
