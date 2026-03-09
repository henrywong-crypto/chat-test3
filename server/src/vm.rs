use std::path::{Path, PathBuf};

use aws_config::default_provider::credentials::DefaultCredentialsChain;
use aws_credential_types::provider::ProvideCredentials;
use firecracker_manager::{build_mmds_with_iam, system_time_to_iso8601, ImdsCredential, VmConfig};
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::state::{AppState, VmEntry};

pub(crate) fn build_vm_config(
    state: &AppState,
    iam_creds: Option<(String, ImdsCredential)>,
    user_rootfs: Option<&Path>,
) -> VmConfig {
    let vm_id = Uuid::new_v4().to_string();
    let (mmds_metadata, mmds_imds_compat) = match iam_creds {
        Some((role_name, cred)) => (build_mmds_with_iam(&vm_id, &role_name, &cred), true),
        None => (
            serde_json::json!({ "latest": { "meta-data": { "instance-id": &vm_id } } }),
            false,
        ),
    };
    VmConfig {
        id: vm_id,
        socket_dir: state.socket_dir.clone(),
        kernel_path: state.kernel_path.clone(),
        rootfs_path: user_rootfs.map(|p| p.to_path_buf()).unwrap_or_else(|| state.rootfs_path.clone()),
        vcpu_count: 2,
        mem_size_mib: 4096,
        boot_args: "reboot=k panic=1 quiet loglevel=3 selinux=0".to_string(),
        mmds_metadata: Some(mmds_metadata),
        mmds_imds_compat,
    }
}

pub(crate) fn user_rootfs_path(user_rootfs_dir: &Path, email: &str) -> PathBuf {
    let sanitized: String = email.chars().map(|c| if c.is_alphanumeric() { c } else { '_' }).collect();
    user_rootfs_dir.join(format!("{sanitized}.ext4"))
}

pub(crate) fn find_user_rootfs(user_rootfs_dir: &Path, email: &str) -> Option<PathBuf> {
    let path = user_rootfs_path(user_rootfs_dir, email);
    path.exists().then_some(path)
}

pub(crate) async fn fetch_host_iam_credentials() -> Option<(String, ImdsCredential)> {
    let provider = DefaultCredentialsChain::builder().build().await;
    let creds = provider
        .provide_credentials()
        .await
        .map_err(|e| warn!("failed to fetch host credentials: {e}"))
        .ok()?;
    let role_name = std::env::var("AWS_ROLE_NAME").unwrap_or_else(|_| "vm-role".to_string());
    let expiration = creds
        .expiry()
        .map(system_time_to_iso8601)
        .unwrap_or_else(|| "2099-01-01T00:00:00Z".to_string());
    Some((
        role_name,
        ImdsCredential::new(
            creds.access_key_id(),
            creds.secret_access_key(),
            creds.session_token().unwrap_or(""),
            expiration,
        ),
    ))
}

pub(crate) async fn save_all_vm_rootfs(app_state: &AppState) {
    let entries: Vec<(String, VmEntry)> = {
        let Ok(mut registry) = app_state.vms.lock() else { return };
        registry.drain().collect()
    };
    if entries.is_empty() {
        return;
    }
    info!("saving rootfs for {} running vm(s) before shutdown", entries.len());
    if let Err(e) = tokio::fs::create_dir_all(&app_state.user_rootfs_dir).await {
        error!("failed to create user rootfs dir on shutdown: {e}");
        return;
    }
    for (vm_id, entry) in entries {
        let user_rootfs = user_rootfs_path(&app_state.user_rootfs_dir, &entry.email);
        info!(email = %entry.email, vm_id = %vm_id, dest = %user_rootfs.display(), "saving rootfs on shutdown");
        if let Err(e) = entry._guard.save_rootfs_to(&user_rootfs).await {
            error!(email = %entry.email, vm_id = %vm_id, "failed to save rootfs on shutdown: {e}");
        }
    }
}
