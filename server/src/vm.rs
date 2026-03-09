use std::path::{Path, PathBuf};

use aws_config::default_provider::credentials::DefaultCredentialsChain;
use aws_credential_types::provider::ProvideCredentials;
use firecracker_manager::{build_mmds_with_iam, system_time_to_iso8601, ImdsCredential, VmConfig};
use tracing::warn;
use uuid::Uuid;

use crate::state::AppState;

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
