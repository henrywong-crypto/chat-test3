use anyhow::Result;
use firecracker_manager::{
    build_mmds_with_iam, put_mmds, JailerConfig, Vm, VmConfig,
};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tracing::{info, warn};
use uuid::Uuid;

mod iam;
mod rootfs;

pub use iam::{fetch_host_iam_credentials, HostIamCredential};
pub use rootfs::{
    build_user_rootfs_path, ensure_user_rootfs, find_user_rootfs, save_all_vm_rootfs,
};

pub type VmRegistry = Arc<Mutex<HashMap<String, VmEntry>>>;

pub struct VmEntry {
    pub user_id: Uuid,
    pub has_iam_creds: bool,
    pub created_at: Instant,
    pub ws_connected: bool,
    pub vm: Vm,
}

pub struct VmBuildConfig {
    pub kernel_path: PathBuf,
    pub rootfs_path: PathBuf,
    pub net_helper_path: PathBuf,
    pub vcpu_count: u8,
    pub mem_size_mib: u32,
    pub jailer_path: PathBuf,
    pub firecracker_path: PathBuf,
    pub jailer_uid: u32,
    pub jailer_gid: u32,
    pub jailer_chroot_base: PathBuf,
}

pub fn build_vm_config(
    vm_build_config: &VmBuildConfig,
    iam_creds: HostIamCredential,
    user_rootfs: Option<&Path>,
) -> Result<VmConfig> {
    let vm_id = Uuid::new_v4().to_string();
    let mmds_metadata =
        build_mmds_with_iam(&vm_id, &iam_creds.role_name, &iam_creds.credential)?;
    info!("configured mmds");
    Ok(VmConfig {
        id: vm_id,
        kernel_path: vm_build_config.kernel_path.clone(),
        rootfs_path: user_rootfs
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| vm_build_config.rootfs_path.clone()),
        net_helper_path: vm_build_config.net_helper_path.clone(),
        vcpu_count: vm_build_config.vcpu_count,
        mem_size_mib: vm_build_config.mem_size_mib,
        boot_args: "reboot=k panic=1 quiet loglevel=3 selinux=0 8250.nr_uarts=0".to_string(),
        mmds_metadata: Some(mmds_metadata),
        mmds_imds_compat: true,
        jailer: JailerConfig {
            jailer_path: vm_build_config.jailer_path.clone(),
            firecracker_path: vm_build_config.firecracker_path.clone(),
            uid: vm_build_config.jailer_uid,
            gid: vm_build_config.jailer_gid,
            chroot_base: vm_build_config.jailer_chroot_base.clone(),
        },
    })
}

const CONNECT_TIMEOUT: Duration = Duration::from_secs(60);

pub async fn refresh_all_vm_mmds(vms: &VmRegistry, use_iam_creds: bool, iam_role_name: &str) {
    if !use_iam_creds {
        return;
    }
    let Some(host_iam_credential) = fetch_host_iam_credentials(iam_role_name).await
        .map_err(|e| warn!("failed to fetch host IAM credentials: {e}"))
        .ok() else {
        return;
    };
    let vm_socket_paths: HashMap<String, PathBuf> = {
        let Ok(registry) = vms.lock() else {
            return;
        };
        registry
            .iter()
            .filter(|(_, e)| e.has_iam_creds)
            .map(|(vm_id, e)| (vm_id.clone(), e.vm.socket_path()))
            .collect()
    };
    for (vm_id, socket_path) in vm_socket_paths {
        refresh_vm_mmds(&vm_id, &socket_path, &host_iam_credential)
            .await
            .unwrap_or_else(|e| warn!("failed to refresh mmds: {e}"));
    }
}

async fn refresh_vm_mmds(
    vm_id: &str,
    socket_path: &Path,
    host_iam_credential: &HostIamCredential,
) -> Result<()> {
    let metadata =
        build_mmds_with_iam(vm_id, &host_iam_credential.role_name, &host_iam_credential.credential)?;
    put_mmds(socket_path, &metadata).await
}

pub async fn sweep_idle_vms(vms: &VmRegistry) {
    let _stale_vms: Vec<VmEntry> = {
        let Ok(mut registry) = vms.lock() else {
            return;
        };
        let stale_ids: Vec<String> = registry
            .iter()
            .filter(|(_, e)| !e.ws_connected && e.created_at.elapsed() > CONNECT_TIMEOUT)
            .map(|(id, _)| id.clone())
            .collect();
        stale_ids
            .into_iter()
            .filter_map(|id| registry.remove(&id))
            .collect()
    };
}
