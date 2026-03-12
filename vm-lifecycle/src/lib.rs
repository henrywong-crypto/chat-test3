use anyhow::Result;
use aws_config::default_provider::credentials::DefaultCredentialsChain;
use aws_credential_types::{provider::ProvideCredentials, Credentials};
use firecracker_manager::{build_mmds_with_iam, put_mmds, ImdsCredential, JailerConfig, Vm, VmConfig};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime},
};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};
use tokio::sync::Mutex as AsyncMutex;
use tracing::{error, info, warn};
use uuid::Uuid;

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

fn system_time_to_iso8601(t: SystemTime) -> String {
    OffsetDateTime::try_from(t)
        .ok()
        .and_then(|dt| dt.format(&Rfc3339).ok())
        .unwrap_or_else(|| "2099-01-01T00:00:00Z".to_string())
}

pub fn build_vm_config(
    vm_build_config: &VmBuildConfig,
    iam_creds: Option<(String, ImdsCredential)>,
    user_rootfs: Option<&Path>,
) -> Result<VmConfig> {
    let vm_id = Uuid::new_v4().to_string();
    let (mmds_metadata, mmds_imds_compat) = match iam_creds {
        Some((role_name, cred)) => (build_mmds_with_iam(&vm_id, &role_name, &cred)?, true),
        None => (
            serde_json::json!({ "latest": { "meta-data": { "instance-id": &vm_id } } }),
            false,
        ),
    };
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
        mmds_imds_compat,
        jailer: JailerConfig {
            jailer_path: vm_build_config.jailer_path.clone(),
            firecracker_path: vm_build_config.firecracker_path.clone(),
            uid: vm_build_config.jailer_uid,
            gid: vm_build_config.jailer_gid,
            chroot_base: vm_build_config.jailer_chroot_base.clone(),
        },
    })
}

pub fn build_user_rootfs_path(user_rootfs_dir: &Path, user_id: Uuid) -> PathBuf {
    user_rootfs_dir.join(format!("{user_id}.ext4"))
}

pub fn find_user_rootfs(user_rootfs_dir: &Path, user_id: Uuid) -> Option<PathBuf> {
    let rootfs_path = build_user_rootfs_path(user_rootfs_dir, user_id);
    rootfs_path.exists().then_some(rootfs_path)
}

pub async fn ensure_user_rootfs(
    user_rootfs_dir: &Path,
    base_rootfs_path: &Path,
    user_id: Uuid,
    rootfs_lock: &AsyncMutex<()>,
) -> Result<PathBuf> {
    let rootfs_path = build_user_rootfs_path(user_rootfs_dir, user_id);
    let _guard = rootfs_lock.lock().await;
    if rootfs_path.exists() {
        return Ok(rootfs_path);
    }
    tokio::fs::create_dir_all(user_rootfs_dir).await?;
    tokio::fs::copy(base_rootfs_path, &rootfs_path).await?;
    Ok(rootfs_path)
}

pub async fn fetch_host_iam_credentials() -> Option<(String, ImdsCredential)> {
    let credentials_chain = DefaultCredentialsChain::builder().build().await;
    let credentials = credentials_chain
        .provide_credentials()
        .await
        .map_err(|e| warn!("failed to fetch host credentials: {e}"))
        .ok()?;
    let role_name = std::env::var("AWS_ROLE_NAME").unwrap_or_else(|_| "vm-role".to_string());
    let expiration = credentials
        .expiry()
        .map(system_time_to_iso8601)
        .unwrap_or_else(|| "2099-01-01T00:00:00Z".to_string());
    Some((role_name, build_imds_credential(&credentials, expiration)))
}

fn build_imds_credential(credentials: &Credentials, expiration: String) -> ImdsCredential {
    ImdsCredential::new(
        credentials.access_key_id(),
        credentials.secret_access_key(),
        credentials.session_token().unwrap_or(""),
        expiration,
    )
}

pub async fn refresh_all_vm_mmds(vms: &VmRegistry) {
    let Some((role_name, cred)) = fetch_host_iam_credentials().await else {
        return;
    };
    let vm_targets: Vec<(String, PathBuf)> = {
        let Ok(registry) = vms.lock() else {
            return;
        };
        registry
            .iter()
            .filter(|(_, e)| e.has_iam_creds)
            .map(|(vm_id, e)| (vm_id.clone(), e.vm.socket_path()))
            .collect()
    };
    for (vm_id, socket_path) in vm_targets {
        let metadata = match build_mmds_with_iam(&vm_id, &role_name, &cred) {
            Ok(metadata) => metadata,
            Err(e) => {
                warn!(vm_id = %vm_id, "failed to build mmds metadata for refresh: {e}");
                continue;
            }
        };
        if let Err(e) = put_mmds(&socket_path, &metadata).await {
            warn!(vm_id = %vm_id, "failed to refresh mmds: {e}");
        }
    }
}

const CONNECT_TIMEOUT: Duration = Duration::from_secs(60);

pub async fn sweep_idle_vms(vms: &VmRegistry) {
    let stale_entries: Vec<(String, VmEntry)> = {
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
            .filter_map(|id| registry.remove(&id).map(|e| (id, e)))
            .collect()
    };
    for (vm_id, _) in stale_entries {
        info!(vm_id = %vm_id, "dropping idle vm (no websocket connected within timeout)");
    }
}

pub async fn save_all_vm_rootfs(
    vms: &VmRegistry,
    user_rootfs_dir: &Path,
    rootfs_lock: &AsyncMutex<()>,
) {
    let vm_entries: Vec<(String, VmEntry)> = {
        let Ok(mut registry) = vms.lock() else {
            return;
        };
        registry.drain().collect()
    };
    if vm_entries.is_empty() {
        return;
    }
    info!(
        "saving rootfs for {} running vm(s) before shutdown",
        vm_entries.len()
    );
    if let Err(e) = tokio::fs::create_dir_all(user_rootfs_dir).await {
        error!("failed to create user rootfs dir on shutdown: {e}");
        return;
    }
    let _guard = rootfs_lock.lock().await;
    for (vm_id, vm_entry) in vm_entries {
        let rootfs_path = build_user_rootfs_path(user_rootfs_dir, vm_entry.user_id);
        info!(user_id = %vm_entry.user_id, vm_id = %vm_id, dest = %rootfs_path.display(), "saving rootfs on shutdown");
        if let Err(e) = vm_entry.vm.save_rootfs(&rootfs_path).await {
            error!(user_id = %vm_entry.user_id, vm_id = %vm_id, "failed to save rootfs on shutdown: {e}");
        }
    }
}
