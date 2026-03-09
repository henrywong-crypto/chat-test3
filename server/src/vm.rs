use anyhow::Result;
use aws_config::default_provider::credentials::DefaultCredentialsChain;
use aws_credential_types::{provider::ProvideCredentials, Credentials};
use chrono::{DateTime, Utc};
use firecracker_manager::{build_mmds_with_iam, put_mmds, ImdsCredential, VmConfig};
use std::{
    path::{Path, PathBuf},
    time::SystemTime,
};
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::state::{AppConfig, AppState, VmEntry};

fn system_time_to_iso8601(t: SystemTime) -> String {
    let dt: DateTime<Utc> = t.into();
    dt.format("%Y-%m-%dT%H:%M:%SZ").to_string()
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
        socket_dir: state.socket_dir.clone(),
        kernel_path: state.kernel_path.clone(),
        rootfs_path: user_rootfs
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| state.rootfs_path.clone()),
        net_helper_path: state.net_helper_path.clone(),
        vcpu_count: 2,
        mem_size_mib: 4096,
        boot_args: "reboot=k panic=1 quiet loglevel=3 selinux=0".to_string(),
        mmds_metadata: Some(mmds_metadata),
        mmds_imds_compat,
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
) -> Result<PathBuf> {
    let rootfs_path = user_rootfs_path(user_rootfs_dir, user_id);
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
            .map(|(vm_id, e)| (vm_id.clone(), e._guard.socket_path().to_path_buf()))
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
        save_vm_entry_rootfs(&app_state.user_rootfs_dir, &vm_id, vm_entry).await;
    }
}

async fn save_vm_entry_rootfs(user_rootfs_dir: &Path, vm_id: &str, vm_entry: VmEntry) {
    let rootfs_path = user_rootfs_path(user_rootfs_dir, vm_entry.user_id);
    info!(user_id = %vm_entry.user_id, vm_id = %vm_id, dest = %rootfs_path.display(), "saving rootfs on shutdown");
    if let Err(e) = vm_entry._guard.save_rootfs_to(&rootfs_path).await {
        error!(user_id = %vm_entry.user_id, vm_id = %vm_id, "failed to save rootfs on shutdown: {e}");
    }
}
