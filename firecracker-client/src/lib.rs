use anyhow::{bail, Context, Result};
use http_body_util::{BodyExt, Full};
use hyper::{body::Bytes, client::conn::http1, Method, Request};
use hyper_util::rt::TokioIo;
use serde::Serialize;
use std::path::Path;
use tokio::net::UnixStream;

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

#[derive(Serialize)]
pub struct Drive {
    pub drive_id: String,
    pub path_on_host: String,
    pub is_root_device: bool,
    pub is_read_only: bool,
}

pub async fn set_machine_config(socket_path: &Path, machine_config: &MachineConfig) -> Result<()> {
    let body = serde_json::to_vec(machine_config)?;
    send_put(socket_path, "/machine-config", body).await
}

pub async fn set_boot_source(socket_path: &Path, boot_source: &BootSource) -> Result<()> {
    let body = serde_json::to_vec(boot_source)?;
    send_put(socket_path, "/boot-source", body).await
}

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


pub async fn set_drive(socket_path: &Path, drive: &Drive) -> Result<()> {
    let path = format!("/drives/{}", drive.drive_id);
    let body = serde_json::to_vec(drive)?;
    send_put(socket_path, &path, body).await
}

pub async fn start_instance(socket_path: &Path) -> Result<()> {
    let body = br#"{"action_type":"InstanceStart"}"#.to_vec();
    send_put(socket_path, "/actions", body).await
}

#[derive(Serialize)]
pub struct MmdsConfig {
    /// MMDS version. Use "V2" for IMDSv2-style token-based requests (recommended for AWS SDK).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub network_interfaces: Vec<String>,
    /// IPv4 address for MMDS in the guest. Default 169.254.169.254 (same as EC2 IMDS).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ipv4_address: Option<String>,
    /// When true, MMDS responds in EC2 IMDS format so AWS SDK default credential chain works.
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

async fn send_put(socket_path: &Path, uri: &str, body: Vec<u8>) -> Result<()> {
    send_request(socket_path, Method::PUT, uri, body).await
}

async fn send_request(socket_path: &Path, method: Method, uri: &str, body: Vec<u8>) -> Result<()> {
    let stream = UnixStream::connect(socket_path).await.with_context(|| {
        format!(
            "failed to connect to firecracker socket {}",
            socket_path.display()
        )
    })?;
    let (mut sender, conn) = http1::handshake(TokioIo::new(stream)).await?;
    tokio::spawn(conn);

    let request = Request::builder()
        .method(method)
        .uri(uri)
        .header("Host", "localhost")
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .body(Full::new(Bytes::from(body)))?;

    let response = sender.send_request(request).await?;

    if !response.status().is_success() {
        let status = response.status();
        let bytes = response.into_body().collect().await?.to_bytes();
        let body = String::from_utf8_lossy(&bytes).into_owned();
        bail!("firecracker api returned {status}: {body}");
    }

    Ok(())
}
