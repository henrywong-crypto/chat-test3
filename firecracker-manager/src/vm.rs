use anyhow::{Context, Result};
use firecracker_client::start_instance;
use nix::{
    sys::signal::{kill, Signal},
    unistd::Pid,
};
use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicU32, Ordering},
};
use tracing::info;

use crate::configure::configure_vm;
use crate::network::{create_tap, delete_tap, format_guest_ip, format_guest_mac, format_tap_ip, format_tap_name};
use crate::process::{
    build_vm_boot_args, build_vm_file_paths, check_still_running, copy_rootfs, spawn_firecracker,
    wait_for_socket, write_vm_meta,
};

pub(crate) static VM_NET_COUNTER: AtomicU32 = AtomicU32::new(0);

pub struct VmConfig {
    pub id: String,
    pub socket_dir: PathBuf,
    pub kernel_path: PathBuf,
    pub rootfs_path: PathBuf,
    pub vcpu_count: u8,
    pub mem_size_mib: u32,
    pub boot_args: String,
    pub mmds_metadata: Option<serde_json::Value>,
    pub mmds_imds_compat: bool,
}

pub struct VmGuard {
    pub id: String,
    pub guest_ip: String,
    pub pid: u32,
    tap_name: String,
    rootfs_copy: PathBuf,
    socket_path: PathBuf,
    meta_path: PathBuf,
}

impl VmGuard {
    pub fn delete(self) {
    }

    pub async fn save_rootfs_to(&self, dest: &Path) -> Result<()> {
        if tokio::fs::rename(&self.rootfs_copy, dest).await.is_err() {
            tokio::fs::copy(&self.rootfs_copy, dest)
                .await
                .with_context(|| format!("failed to copy rootfs to {}", dest.display()))?;
        }
        Ok(())
    }
}

impl Drop for VmGuard {
    fn drop(&mut self) {
        let _ = kill(Pid::from_raw(self.pid as i32), Signal::SIGTERM);
        delete_tap(&self.tap_name);
        let _ = std::fs::remove_file(&self.rootfs_copy);
        let _ = std::fs::remove_file(&self.socket_path);
        let _ = std::fs::remove_file(&self.meta_path);
    }
}

pub async fn create_vm(vm_config: &VmConfig) -> Result<VmGuard> {
    let net_idx = VM_NET_COUNTER.fetch_add(1, Ordering::Relaxed) % 254;
    let tap_name = format_tap_name(net_idx);
    let tap_ip = format_tap_ip(net_idx);
    let mac = format_guest_mac(net_idx);
    let guest_ip = format_guest_ip(net_idx);
    let (socket_path, rootfs_copy, meta_path) =
        build_vm_file_paths(&vm_config.socket_dir, &vm_config.id);
    let boot_args = build_vm_boot_args(&vm_config.boot_args, &guest_ip, net_idx);

    create_tap(&tap_name, &tap_ip).await?;
    info!(src = %vm_config.rootfs_path.display(), dst = %rootfs_copy.display(), "copying rootfs");
    copy_rootfs(&vm_config.rootfs_path, &rootfs_copy).await?;
    let mut child = spawn_firecracker(&socket_path)?;
    let pid = child
        .id()
        .context("process exited before pid was available")?;
    write_vm_meta(&meta_path, pid, &tap_name, &rootfs_copy);
    wait_for_socket(&socket_path).await?;
    configure_vm(
        &socket_path,
        &rootfs_copy,
        vm_config,
        &tap_name,
        &mac,
        &boot_args,
    )
    .await?;
    start_instance(&socket_path).await?;
    check_still_running(&mut child).await?;

    Ok(VmGuard {
        id: vm_config.id.clone(),
        guest_ip,
        pid,
        tap_name,
        rootfs_copy,
        socket_path,
        meta_path,
    })
}
