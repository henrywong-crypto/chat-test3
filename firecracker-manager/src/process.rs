use anyhow::{bail, Result};
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::process::{Child, Command};

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
    for _ in 0..50 {
        if socket_path.exists() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    bail!("timed out waiting for firecracker socket")
}

pub(crate) async fn check_still_running(child: &mut Child) -> Result<()> {
    tokio::time::sleep(Duration::from_millis(500)).await;
    match child.try_wait()? {
        Some(status) => bail!("firecracker exited immediately after start: {status}"),
        None => Ok(()),
    }
}

pub(crate) fn write_vm_meta(meta_path: &Path, pid: u32, tap_name: &str, rootfs_copy: &Path) {
    let content = format!("{pid}\n{tap_name}\n{}", rootfs_copy.display());
    let _ = std::fs::write(meta_path, content);
}

pub(crate) fn build_vm_file_paths(socket_dir: &Path, vm_id: &str) -> (PathBuf, PathBuf, PathBuf) {
    let socket_path = socket_dir.join(format!("fc-{vm_id}.socket"));
    let rootfs_copy = socket_dir.join(format!("rootfs-{vm_id}.ext4"));
    let meta_path = socket_dir.join(format!("fc-{vm_id}.meta"));
    (socket_path, rootfs_copy, meta_path)
}

pub(crate) fn build_vm_boot_args(base_boot_args: &str, guest_ip: &str, net_idx: u32) -> String {
    format!("{base_boot_args} ip={guest_ip}::172.16.{net_idx}.1:255.255.255.252::eth0:none")
}
