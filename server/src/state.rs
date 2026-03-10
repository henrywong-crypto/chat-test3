use anyhow::Result;
use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};
use config::{Config, Environment, File};
use firecracker_manager::Vm;
use serde::Deserialize;
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Instant,
};
use store::PgPool;
use tokio::sync::Mutex as AsyncMutex;
use tracing::error;
use uuid::Uuid;

#[derive(Clone, Deserialize)]
pub(crate) struct AppConfig {
    #[serde(default = "default_kernel_path")]
    pub(crate) kernel_path: PathBuf,
    #[serde(default = "default_rootfs_path")]
    pub(crate) rootfs_path: PathBuf,
    #[serde(default = "default_net_helper_path")]
    pub(crate) net_helper_path: PathBuf,
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
    #[serde(default = "default_database_url")]
    pub(crate) database_url: String,
    #[serde(default = "default_port")]
    pub(crate) port: u16,
    #[serde(default = "default_jailer_path")]
    pub(crate) jailer_path: PathBuf,
    #[serde(default = "default_firecracker_path")]
    pub(crate) firecracker_path: PathBuf,
    #[serde(default)]
    pub(crate) jailer_uid: u32,
    #[serde(default)]
    pub(crate) jailer_gid: u32,
    #[serde(default = "default_jailer_chroot_base")]
    pub(crate) jailer_chroot_base: PathBuf,
    #[serde(default = "default_vm_vcpu_count")]
    pub(crate) vm_vcpu_count: u8,
    #[serde(default = "default_vm_mem_size_mib")]
    pub(crate) vm_mem_size_mib: u32,
    #[serde(default = "default_vm_max_count")]
    pub(crate) vm_max_count: usize,
}

fn default_kernel_path() -> PathBuf {
    PathBuf::from("/var/lib/fc/vmlinux")
}
fn default_rootfs_path() -> PathBuf {
    PathBuf::from("/var/lib/fc/rootfs.ext4")
}
fn default_net_helper_path() -> PathBuf {
    PathBuf::from("/usr/local/bin/net-helper")
}
fn default_ssh_key_path() -> PathBuf {
    PathBuf::from("/var/lib/fc/id_rsa")
}
fn default_ssh_user() -> String {
    "root".to_string()
}
fn default_vm_host_key_path() -> PathBuf {
    PathBuf::from("/var/lib/fc/vm_host_key.pub")
}
fn default_cognito_redirect_uri() -> String {
    "http://localhost:3000/callback".to_string()
}
fn default_user_rootfs_dir() -> PathBuf {
    PathBuf::from("/home/ubuntu/fc-users")
}
fn default_upload_dir() -> String {
    "/home/ubuntu".to_string()
}
fn default_database_url() -> String {
    "postgres://localhost/webcode".to_string()
}
fn default_port() -> u16 {
    3000
}
fn default_jailer_path() -> PathBuf {
    PathBuf::from("/usr/local/bin/jailer")
}
fn default_firecracker_path() -> PathBuf {
    PathBuf::from("/usr/local/bin/firecracker")
}
fn default_jailer_chroot_base() -> PathBuf {
    PathBuf::from("/srv/jailer")
}
fn default_vm_vcpu_count() -> u8 {
    2
}
fn default_vm_mem_size_mib() -> u32 {
    4096
}
fn default_vm_max_count() -> usize {
    20
}

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
    pub(crate) db: PgPool,
    pub(crate) vms: VmRegistry,
    pub(crate) rootfs_locks: RootfsLocks,
}

impl AppState {
    pub(crate) fn new(config: AppConfig, pg_pool: PgPool) -> Self {
        AppState {
            config,
            db: pg_pool,
            vms: Arc::new(Mutex::new(HashMap::new())),
            rootfs_locks: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl std::ops::Deref for AppState {
    type Target = AppConfig;
    fn deref(&self) -> &AppConfig {
        &self.config
    }
}

pub(crate) type VmRegistry = Arc<Mutex<HashMap<String, VmEntry>>>;
pub(crate) type RootfsLocks = Arc<Mutex<HashMap<Uuid, Arc<AsyncMutex<()>>>>>;

pub(crate) struct VmEntry {
    pub(crate) user_id: Uuid,
    pub(crate) has_iam_creds: bool,
    pub(crate) created_at: Instant,
    pub(crate) ws_connected: Arc<AtomicBool>,
    pub(crate) vm: Vm,
}

pub(crate) struct AppError(pub(crate) anyhow::Error);

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        error!("internal error: {}", self.0);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "An internal error occurred",
        )
            .into_response()
    }
}

impl<E: Into<anyhow::Error>> From<E> for AppError {
    fn from(app_error: E) -> Self {
        AppError(app_error.into())
    }
}

pub(crate) fn mark_vm_ws_connected(vms: &VmRegistry, vm_id: &str) {
    if let Ok(registry) = vms.lock() {
        if let Some(entry) = registry.get(vm_id) {
            entry.ws_connected.store(true, Ordering::Relaxed);
        }
    }
}

pub(crate) fn find_vm_guest_ip_for_user(
    vms: &VmRegistry,
    vm_id: &str,
    user_id: Uuid,
) -> Option<String> {
    let registry = vms.lock().ok()?;
    let vm_entry = registry.get(vm_id)?;
    (vm_entry.user_id == user_id).then(|| vm_entry.vm.guest_ip())
}

pub(crate) fn get_rootfs_lock(locks: &RootfsLocks, user_id: Uuid) -> Arc<AsyncMutex<()>> {
    locks
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entry(user_id)
        .or_insert_with(|| Arc::new(AsyncMutex::new(())))
        .clone()
}
