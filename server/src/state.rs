use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use anyhow::Result;
use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};
use config::{Config, Environment, File};
use firecracker_manager::VmGuard;
use serde::Deserialize;
use tracing::error;

#[derive(Clone, Deserialize)]
pub(crate) struct AppConfig {
    #[serde(default = "default_kernel_path")]
    pub(crate) kernel_path: PathBuf,
    #[serde(default = "default_rootfs_path")]
    pub(crate) rootfs_path: PathBuf,
    #[serde(default = "default_socket_dir")]
    pub(crate) socket_dir: PathBuf,
    #[serde(default = "default_ssh_key_path")]
    pub(crate) ssh_key_path: PathBuf,
    #[serde(default = "default_ssh_user")]
    pub(crate) ssh_user: String,
    #[serde(default = "default_vm_host_key_path")]
    pub(crate) vm_host_key_path: PathBuf,
    #[serde(default)]
    pub(crate) cognito_client_id: String,
    #[serde(default)]
    pub(crate) cognito_client_secret: String,
    #[serde(default)]
    pub(crate) cognito_domain: String,
    #[serde(default = "default_cognito_redirect_uri")]
    pub(crate) cognito_redirect_uri: String,
    #[serde(default)]
    pub(crate) cognito_region: String,
    #[serde(default)]
    pub(crate) cognito_user_pool_id: String,
    #[serde(default = "default_user_rootfs_dir")]
    pub(crate) user_rootfs_dir: PathBuf,
    #[serde(default = "default_upload_dir")]
    pub(crate) upload_dir: String,
    #[serde(default = "default_max_vms_per_user")]
    pub(crate) max_vms_per_user: usize,
    #[serde(default = "default_port")]
    pub(crate) port: u16,
}

fn default_kernel_path() -> PathBuf        { PathBuf::from("/var/lib/fc/vmlinux") }
fn default_rootfs_path() -> PathBuf        { PathBuf::from("/var/lib/fc/rootfs.ext4") }
fn default_socket_dir() -> PathBuf         { PathBuf::from("/tmp") }
fn default_ssh_key_path() -> PathBuf       { PathBuf::from("/var/lib/fc/id_rsa") }
fn default_ssh_user() -> String            { "root".to_string() }
fn default_vm_host_key_path() -> PathBuf   { PathBuf::from("/var/lib/fc/vm_host_key.pub") }
fn default_cognito_redirect_uri() -> String { "http://localhost:3000/callback".to_string() }
fn default_user_rootfs_dir() -> PathBuf    { PathBuf::from("/home/ubuntu/fc-users") }
fn default_upload_dir() -> String          { "/home/ubuntu".to_string() }
fn default_max_vms_per_user() -> usize     { 2 }
fn default_port() -> u16                   { 3000 }

pub(crate) fn load_config() -> Result<AppConfig> {
    let app_config = Config::builder()
        .add_source(File::with_name("config").required(false))
        .add_source(Environment::default())
        .build()?
        .try_deserialize()?;
    Ok(app_config)
}

#[derive(Clone)]
pub(crate) struct AppState {
    pub(crate) config: AppConfig,
    pub(crate) vms: VmRegistry,
}

impl AppState {
    pub(crate) fn new(config: AppConfig) -> Self {
        AppState { config, vms: Arc::new(Mutex::new(HashMap::new())) }
    }
}

impl std::ops::Deref for AppState {
    type Target = AppConfig;
    fn deref(&self) -> &AppConfig {
        &self.config
    }
}

pub(crate) type VmRegistry = Arc<Mutex<HashMap<String, VmEntry>>>;

pub(crate) struct VmEntry {
    pub(crate) guest_ip: String,
    pub(crate) pid: u32,
    pub(crate) created_at: u64,
    pub(crate) email: String,
    pub(crate) _guard: VmGuard,
}

#[derive(serde::Serialize)]
pub(crate) struct VmInfo {
    pub(crate) id: String,
    pub(crate) guest_ip: String,
    pub(crate) pid: u32,
    pub(crate) created_at: u64,
}

pub(crate) struct AppError(pub(crate) anyhow::Error);

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        error!("internal error: {}", self.0);
        (StatusCode::INTERNAL_SERVER_ERROR, "An internal error occurred").into_response()
    }
}

impl<E: Into<anyhow::Error>> From<E> for AppError {
    fn from(app_error: E) -> Self {
        AppError(app_error.into())
    }
}

pub(crate) fn find_vm_guest_ip_for_user(vms: &VmRegistry, vm_id: &str, email: &str) -> Option<String> {
    let registry = vms.lock().ok()?;
    let entry = registry.get(vm_id)?;
    (entry.email == email).then(|| entry.guest_ip.clone())
}
