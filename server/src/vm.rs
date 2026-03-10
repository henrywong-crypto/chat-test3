use anyhow::Result;
use std::{
    path::{Path, PathBuf},
    sync::atomic::Ordering,
    time::{Duration, SystemTime},
};
use aws_config::default_provider::credentials::DefaultCredentialsChain;
use aws_credential_types::{provider::ProvideCredentials, Credentials};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};
use firecracker_manager::{build_mmds_with_iam, put_mmds, ImdsCredential, JailerConfig, Vm, VmConfig};
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::state::{get_rootfs_lock, AppConfig, AppState, RootfsLocks, VmEntry};

fn system_time_to_iso8601(t: SystemTime) -> String {
    OffsetDateTime::try_from(t)
        .ok()
        .and_then(|dt| dt.format(&Rfc3339).ok())
        .unwrap_or_else(|| "2099-01-01T00:00:00Z".to_string())
}

pub(crate) fn build_vm_config(
    state: &AppConfig,
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
        kernel_path: state.kernel_path.clone(),
        rootfs_path: user_rootfs
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| state.rootfs_path.clone()),
        net_helper_path: state.net_helper_path.clone(),
        vcpu_count: 2,
        mem_size_mib: 4096,
        boot_args: "reboot=k panic=1 quiet loglevel=3 selinux=0 8250.nr_uarts=0".to_string(),
        mmds_metadata: Some(mmds_metadata),
        mmds_imds_compat,
        jailer: JailerConfig {
            jailer_path: state.jailer_path.clone(),
            firecracker_path: state.firecracker_path.clone(),
            uid: state.jailer_uid,
            gid: state.jailer_gid,
            chroot_base: state.jailer_chroot_base.clone(),
        },
    })
}

pub(crate) fn user_rootfs_path(user_rootfs_dir: &Path, user_id: Uuid) -> PathBuf {
    user_rootfs_dir.join(format!("{user_id}.ext4"))
}

pub(crate) fn find_user_rootfs(user_rootfs_dir: &Path, user_id: Uuid) -> Option<PathBuf> {
    let rootfs_path = user_rootfs_path(user_rootfs_dir, user_id);
    rootfs_path.exists().then_some(rootfs_path)
}

pub(crate) async fn ensure_user_rootfs(
    user_rootfs_dir: &Path,
    base_rootfs_path: &Path,
    user_id: Uuid,
    rootfs_locks: &RootfsLocks,
) -> Result<PathBuf> {
    let rootfs_path = user_rootfs_path(user_rootfs_dir, user_id);
    let lock = get_rootfs_lock(rootfs_locks, user_id);
    let _guard = lock.lock().await;
    if rootfs_path.exists() {
        return Ok(rootfs_path);
    }
    tokio::fs::create_dir_all(user_rootfs_dir).await?;
    tokio::fs::copy(base_rootfs_path, &rootfs_path).await?;
    Ok(rootfs_path)
}

pub(crate) async fn fetch_host_iam_credentials() -> Option<(String, ImdsCredential)> {
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

pub(crate) async fn refresh_all_vm_mmds(app_state: &AppState) {
    let Some((role_name, cred)) = fetch_host_iam_credentials().await else {
        return;
    };
    let vm_targets: Vec<(String, PathBuf)> = {
        let Ok(registry) = app_state.vms.lock() else {
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

pub(crate) async fn sweep_idle_vms(app_state: &AppState) {
    let stale_entries: Vec<(String, VmEntry)> = collect_stale_vm_entries(app_state);
    for (vm_id, _) in stale_entries {
        info!(vm_id = %vm_id, "dropping idle vm (no websocket connected within timeout)");
    }
}

fn collect_stale_vm_entries(app_state: &AppState) -> Vec<(String, VmEntry)> {
    let Ok(mut registry) = app_state.vms.lock() else {
        return Vec::new();
    };
    let stale_ids: Vec<String> = registry
        .iter()
        .filter(|(_, e)| {
            !e.ws_connected.load(Ordering::Relaxed) && e.created_at.elapsed() > CONNECT_TIMEOUT
        })
        .map(|(id, _)| id.clone())
        .collect();
    stale_ids
        .into_iter()
        .filter_map(|id| registry.remove(&id).map(|e| (id, e)))
        .collect()
}

pub(crate) async fn save_all_vm_rootfs(app_state: &AppState) {
    let vm_entries: Vec<(String, VmEntry)> = {
        let Ok(mut registry) = app_state.vms.lock() else {
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
    if let Err(e) = tokio::fs::create_dir_all(&app_state.user_rootfs_dir).await {
        error!("failed to create user rootfs dir on shutdown: {e}");
        return;
    }
    for (vm_id, vm_entry) in vm_entries {
        save_vm_entry_rootfs(
            &app_state.user_rootfs_dir,
            &vm_id,
            vm_entry,
            &app_state.rootfs_locks,
        )
        .await;
    }
}

async fn save_vm_entry_rootfs(
    user_rootfs_dir: &Path,
    vm_id: &str,
    vm_entry: VmEntry,
    rootfs_locks: &RootfsLocks,
) {
    let rootfs_path = user_rootfs_path(user_rootfs_dir, vm_entry.user_id);
    let lock = get_rootfs_lock(rootfs_locks, vm_entry.user_id);
    let _guard = lock.lock().await;
    info!(user_id = %vm_entry.user_id, vm_id = %vm_id, dest = %rootfs_path.display(), "saving rootfs on shutdown");
    if let Err(e) = vm_entry.vm.save_rootfs(&rootfs_path).await {
        error!(user_id = %vm_entry.user_id, vm_id = %vm_id, "failed to save rootfs on shutdown: {e}");
    }
}
