use std::path::Path;

use http_body_util::Full;
use hyper::body::Bytes;
use hyper::client::conn::http1;
use hyper::http::Error as HttpError;
use hyper::{Method, Request, StatusCode};
use hyper_util::rt::TokioIo;
use serde::Serialize;
use thiserror::Error;
use tokio::net::UnixStream;

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Hyper(#[from] hyper::Error),
    #[error(transparent)]
    Http(#[from] HttpError),
    #[error("firecracker api returned {0}")]
    Api(StatusCode),
}

pub type Result<T> = std::result::Result<T, Error>;

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
    let body = serde_json::to_vec(machine_config).unwrap();
    send_put(socket_path, "/machine-config", body).await
}

pub async fn set_boot_source(socket_path: &Path, boot_source: &BootSource) -> Result<()> {
    let body = serde_json::to_vec(boot_source).unwrap();
    send_put(socket_path, "/boot-source", body).await
}

pub async fn set_drive(socket_path: &Path, drive: &Drive) -> Result<()> {
    let path = format!("/drives/{}", drive.drive_id);
    let body = serde_json::to_vec(drive).unwrap();
    send_put(socket_path, &path, body).await
}

pub async fn start_instance(socket_path: &Path) -> Result<()> {
    let body = br#"{"action_type":"InstanceStart"}"#.to_vec();
    send_put(socket_path, "/actions", body).await
}

async fn send_put(socket_path: &Path, uri: &str, body: Vec<u8>) -> Result<()> {
    let stream = UnixStream::connect(socket_path).await?;
    let (mut sender, conn) = http1::handshake(TokioIo::new(stream)).await?;
    tokio::spawn(conn);

    let request = Request::builder()
        .method(Method::PUT)
        .uri(uri)
        .header("Host", "localhost")
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .body(Full::new(Bytes::from(body)))?;

    let response = sender.send_request(request).await?;

    if !response.status().is_success() {
        return Err(Error::Api(response.status()));
    }

    Ok(())
}
