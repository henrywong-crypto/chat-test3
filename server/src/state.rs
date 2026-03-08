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
use clap::Parser;
use firecracker_manager::VmGuard;
use serde::Serialize;

#[derive(Parser)]
pub(crate) struct Args {
    #[arg(long, env = "KERNEL_PATH", default_value = "/var/lib/fc/vmlinux")]
    pub(crate) kernel_path: PathBuf,
    #[arg(long, env = "ROOTFS_PATH", default_value = "/var/lib/fc/rootfs.ext4")]
    pub(crate) rootfs_path: PathBuf,
    #[arg(long, env = "SOCKET_DIR", default_value = "/tmp")]
    pub(crate) socket_dir: PathBuf,
    #[arg(long, env = "SSH_KEY_PATH", default_value = "/var/lib/fc/id_rsa")]
    pub(crate) ssh_key_path: PathBuf,
    #[arg(long, env = "SSH_USER", default_value = "root")]
    pub(crate) ssh_user: String,
    #[arg(long, env = "VM_HOST_KEY_PATH", default_value = "/var/lib/fc/vm_host_key.pub")]
    pub(crate) vm_host_key_path: PathBuf,
    #[arg(long, env = "COGNITO_CLIENT_ID", default_value = "")]
    pub(crate) cognito_client_id: String,
    #[arg(long, env = "COGNITO_CLIENT_SECRET", default_value = "")]
    pub(crate) cognito_client_secret: String,
    #[arg(long, env = "COGNITO_DOMAIN", default_value = "")]
    pub(crate) cognito_domain: String,
    #[arg(long, env = "COGNITO_REDIRECT_URI", default_value = "http://localhost:3000/callback")]
    pub(crate) cognito_redirect_uri: String,
    #[arg(long, env = "COGNITO_REGION", default_value = "")]
    pub(crate) cognito_region: String,
    #[arg(long, env = "COGNITO_USER_POOL_ID", default_value = "")]
    pub(crate) cognito_user_pool_id: String,
    #[arg(long, env = "UPLOAD_DIR", default_value = "/home/user/uploads")]
    pub(crate) upload_dir: String,
    #[arg(long, env = "PORT", default_value = "3000")]
    pub(crate) port: u16,
}

#[derive(Clone)]
pub(crate) struct AppState {
    pub(crate) kernel_path: PathBuf,
    pub(crate) rootfs_path: PathBuf,
    pub(crate) socket_dir: PathBuf,
    pub(crate) ssh_key_path: PathBuf,
    pub(crate) ssh_user: String,
    pub(crate) vm_host_key_path: PathBuf,
    pub(crate) cognito_client_id: String,
    pub(crate) cognito_client_secret: String,
    pub(crate) cognito_domain: String,
    pub(crate) cognito_redirect_uri: String,
    pub(crate) cognito_region: String,
    pub(crate) cognito_user_pool_id: String,
    pub(crate) upload_dir: String,
    pub(crate) vms: VmRegistry,
}

pub(crate) type VmRegistry = Arc<Mutex<HashMap<String, VmEntry>>>;

pub(crate) struct VmEntry {
    pub(crate) guest_ip: String,
    pub(crate) pid: u32,
    pub(crate) created_at: u64,
    pub(crate) _guard: VmGuard,
}

#[derive(Serialize)]
pub(crate) struct VmInfo {
    pub(crate) id: String,
    pub(crate) guest_ip: String,
    pub(crate) pid: u32,
    pub(crate) created_at: u64,
}

pub(crate) struct AppError(pub(crate) anyhow::Error);

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        (StatusCode::INTERNAL_SERVER_ERROR, self.0.to_string()).into_response()
    }
}

impl<E: Into<anyhow::Error>> From<E> for AppError {
    fn from(app_error: E) -> Self {
        AppError(app_error.into())
    }
}

pub(crate) fn build_app_state(args: Args) -> Result<AppState> {
    Ok(AppState {
        kernel_path: args.kernel_path,
        rootfs_path: args.rootfs_path,
        socket_dir: args.socket_dir,
        ssh_key_path: args.ssh_key_path,
        ssh_user: args.ssh_user,
        vm_host_key_path: args.vm_host_key_path,
        cognito_client_id: args.cognito_client_id,
        cognito_client_secret: args.cognito_client_secret,
        cognito_domain: args.cognito_domain,
        cognito_redirect_uri: args.cognito_redirect_uri,
        cognito_region: args.cognito_region,
        cognito_user_pool_id: args.cognito_user_pool_id,
        upload_dir: args.upload_dir,
        vms: Arc::new(Mutex::new(HashMap::new())),
    })
}

pub(crate) fn find_vm_guest_ip(vms: &VmRegistry, vm_id: &str) -> Option<String> {
    vms.lock().ok()?.get(vm_id).map(|vm_entry| vm_entry.guest_ip.clone())
}
