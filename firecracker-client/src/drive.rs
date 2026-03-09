use anyhow::Result;
use serde::Serialize;
use std::path::Path;

use crate::http::send_put;

#[derive(Serialize)]
pub struct Drive {
    pub drive_id: String,
    pub path_on_host: String,
    pub is_root_device: bool,
    pub is_read_only: bool,
}

pub async fn set_drive(socket_path: &Path, drive: &Drive) -> Result<()> {
    let path = format!("/drives/{}", drive.drive_id);
    let body = serde_json::to_vec(drive)?;
    send_put(socket_path, &path, body).await
}
