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
    build_chroot_dir, build_vm_boot_args, copy_rootfs,
    prepare_jail_resources, spawn_firecracker_jailed, wait_for_socket,
};

pub(crate) static VM_NET_COUNTER: AtomicU32 = AtomicU32::new(0);

pub struct JailerConfig {
    pub jailer_path: PathBuf,
    pub firecracker_path: PathBuf,
    pub uid: u32,
    pub gid: u32,
    pub chroot_base: PathBuf,
}

pub struct VmConfig {
    pub id: String,
    pub kernel_path: PathBuf,
    pub rootfs_path: PathBuf,
    pub net_helper_path: PathBuf,
    pub vcpu_count: u8,
    pub mem_size_mib: u32,
    pub boot_args: String,
    pub mmds_metadata: Option<serde_json::Value>,
    pub mmds_imds_compat: bool,
    pub jailer: JailerConfig,
}

pub struct VmGuard {
    pub id: String,
    pub guest_ip: String,
    pub pid: u32,
    net_helper_path: PathBuf,
    tap_name: String,
    rootfs_copy: PathBuf,
    socket_path: PathBuf,
    chroot_dir: PathBuf,
}

impl VmGuard {
    pub fn socket_path(&self) -> &Path {
        &self.socket_path
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
        delete_tap(&self.net_helper_path, &self.tap_name);
        let _ = std::fs::remove_dir_all(&self.chroot_dir);
    }
}

pub async fn create_vm(vm_config: &VmConfig) -> Result<VmGuard> {
    let net_idx = VM_NET_COUNTER.fetch_add(1, Ordering::Relaxed) % 254;
    let tap_name = format_tap_name(net_idx);
    let tap_ip = format_tap_ip(net_idx);
    let mac = format_guest_mac(net_idx);
    let guest_ip = format_guest_ip(net_idx);
    let boot_args = build_vm_boot_args(&vm_config.boot_args, &guest_ip, net_idx);

    create_tap(&vm_config.net_helper_path, &tap_name, &tap_ip).await?;

    let chroot_dir = build_chroot_dir(&vm_config.jailer.chroot_base, &vm_config.id);
    let rootfs_copy = chroot_dir.join("rootfs.ext4");
    let socket_path = chroot_dir.join("run/firecracker.socket");
    let kernel_path_in_jail = PathBuf::from("/vmlinux");
    let rootfs_path_in_jail = PathBuf::from("/rootfs.ext4");

    prepare_jail_resources(&chroot_dir, &vm_config.kernel_path).await?;
    info!(src = %vm_config.rootfs_path.display(), dst = %rootfs_copy.display(), "copying rootfs");
    copy_rootfs(&vm_config.rootfs_path, &rootfs_copy).await?;
    let child = spawn_firecracker_jailed(&vm_config.id, &vm_config.jailer)?;
    let pid = child
        .id()
        .context("process exited before pid was available")?;
    wait_for_socket(&socket_path).await?;
    configure_vm(
        &socket_path,
        &rootfs_path_in_jail,
        &kernel_path_in_jail,
        vm_config,
        &tap_name,
        &mac,
        &boot_args,
    )
    .await?;
    start_instance(&socket_path).await?;

    Ok(VmGuard {
        id: vm_config.id.clone(),
        guest_ip,
        pid,
        net_helper_path: vm_config.net_helper_path.clone(),
        tap_name,
        rootfs_copy,
        socket_path,
        chroot_dir,
    })
}
