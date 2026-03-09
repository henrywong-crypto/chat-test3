use anyhow::Result;
use serde::Serialize;
use std::path::Path;

use crate::http::send_put;

#[derive(Serialize)]
pub struct NetworkInterface {
    pub iface_id: String,
    pub guest_mac: String,
    pub host_dev_name: String,
}

pub async fn set_network_interface(socket_path: &Path, iface: &NetworkInterface) -> Result<()> {
    let path = format!("/network-interfaces/{}", iface.iface_id);
    let body = serde_json::to_vec(iface)?;
    send_put(socket_path, &path, body).await
}
