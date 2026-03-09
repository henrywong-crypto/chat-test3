use anyhow::{Context, Result};
use std::{
    io::ErrorKind,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::process::{Child, Command};

use crate::vm::JailerConfig;

pub(crate) fn spawn_firecracker(socket_path: &Path) -> Result<Child> {
    Ok(Command::new("firecracker")
        .args(["--api-sock", &socket_path.to_string_lossy()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .kill_on_drop(false)
        .process_group(0)
        .spawn()?)
}

pub(crate) fn spawn_firecracker_jailed(vm_id: &str, jailer: &JailerConfig) -> Result<Child> {
    Ok(Command::new(&jailer.jailer_path)
        .args([
            "--id",
            vm_id,
            "--exec-file",
            &jailer.firecracker_path.to_string_lossy(),
            "--uid",
            &jailer.uid.to_string(),
            "--gid",
            &jailer.gid.to_string(),
            "--chroot-base-dir",
            &jailer.chroot_base.to_string_lossy(),
            "--",
            "--api-sock",
            "/run/firecracker.socket",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .kill_on_drop(false)
        .process_group(0)
        .spawn()?)
}

pub(crate) async fn copy_rootfs(src: &Path, dst: &Path) -> Result<()> {
    let status = Command::new("cp")
        .args([
            "--sparse=always",
            &src.to_string_lossy(),
            &dst.to_string_lossy(),
        ])
        .status()
        .await?;
    if !status.success() {
        bail!(
            "failed to copy rootfs from {} to {}",
            src.display(),
            dst.display()
        );
    }
    Ok(())
}

pub(crate) async fn wait_for_socket(socket_path: &Path) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if socket_path.exists() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .context("timed out waiting for firecracker socket")
}

pub(crate) async fn check_still_running(child: &mut Child) -> Result<()> {
    tokio::time::sleep(Duration::from_millis(500)).await;
    match child.try_wait()? {
        Some(status) => bail!("firecracker exited immediately after start: {status}"),
        None => Ok(()),
    }
}

pub(crate) fn build_vm_file_paths(socket_dir: &Path, vm_id: &str) -> (PathBuf, PathBuf) {
    let socket_path = socket_dir.join(format!("fc-{vm_id}.socket"));
    let rootfs_copy = socket_dir.join(format!("rootfs-{vm_id}.ext4"));
    (socket_path, rootfs_copy)
}

pub(crate) fn build_vm_boot_args(base_boot_args: &str, guest_ip: &str, net_idx: u32) -> String {
    format!("{base_boot_args} ip={guest_ip}::172.16.{net_idx}.1:255.255.255.252::eth0:none")
}

pub(crate) fn build_chroot_dir(chroot_base: &Path, vm_id: &str) -> PathBuf {
    chroot_base.join("firecracker").join(vm_id).join("root")
}

pub(crate) async fn prepare_jail_resources(chroot_dir: &Path, kernel_src: &Path) -> Result<()> {
    tokio::fs::create_dir_all(chroot_dir.join("run")).await?;
    let kernel_dst = chroot_dir.join("vmlinux");
    if let Err(e) = tokio::fs::hard_link(kernel_src, &kernel_dst).await {
        if e.kind() == ErrorKind::CrossesDevices {
            tokio::fs::copy(kernel_src, &kernel_dst)
                .await
                .with_context(|| {
                    format!(
                        "failed to copy kernel from {} to {}",
                        kernel_src.display(),
                        kernel_dst.display()
                    )
                })?;
        } else {
            return Err(e).with_context(|| {
                format!(
                    "failed to hard-link kernel from {} to {}",
                    kernel_src.display(),
                    kernel_dst.display()
                )
            });
        }
    }
    Ok(())
}
