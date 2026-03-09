mod mmds_iam;
use anyhow::{bail, Context, Result};
use firecracker_client::{
    put_mmds, set_boot_source, set_drive, set_machine_config, set_mmds_config,
    set_network_interface, start_instance, BootSource, Drive, MachineConfig, MmdsConfig,
    NetworkInterface,
};
pub use mmds_iam::{build_mmds_with_iam, ImdsCredential};
use nix::{
    sys::signal::{kill, Signal},
    unistd::Pid,
};
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    sync::atomic::{AtomicU32, Ordering},
    time::Duration,
};
use tokio::process::{Child, Command};
use tracing::{info, warn};

static VM_NET_COUNTER: AtomicU32 = AtomicU32::new(0);

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

pub struct Vm {
    pub id: String,
    pub guest_ip: String,
    pub socket_path: PathBuf,
    pub pid: u32,
    _child: Child,
    tap_name: String,
    rootfs_copy: PathBuf,
    meta_path: PathBuf,
}

pub struct VmGuard {
    pub id: String,
    pub guest_ip: String,
    pub socket_path: PathBuf,
    pub pid: u32,
    tap_name: String,
    rootfs_copy: PathBuf,
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

impl Vm {
    pub fn into_guard(self) -> VmGuard {
        VmGuard {
            id: self.id,
            guest_ip: self.guest_ip,
            socket_path: self.socket_path,
            pid: self.pid,
            tap_name: self.tap_name,
            rootfs_copy: self.rootfs_copy,
            meta_path: self.meta_path,
        }
    }
}

pub async fn create_vm(vm_config: &VmConfig) -> Result<Vm> {
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

    Ok(Vm {
        id: vm_config.id.clone(),
        guest_ip,
        socket_path,
        pid,
        _child: child,
        tap_name,
        rootfs_copy,
        meta_path,
    })
}

fn build_vm_file_paths(socket_dir: &Path, vm_id: &str) -> (PathBuf, PathBuf, PathBuf) {
    let socket_path = socket_dir.join(format!("fc-{vm_id}.socket"));
    let rootfs_copy = socket_dir.join(format!("rootfs-{vm_id}.ext4"));
    let meta_path = socket_dir.join(format!("fc-{vm_id}.meta"));
    (socket_path, rootfs_copy, meta_path)
}

fn build_vm_boot_args(base_boot_args: &str, guest_ip: &str, net_idx: u32) -> String {
    format!("{base_boot_args} ip={guest_ip}::172.16.{net_idx}.1:255.255.255.252::eth0:none")
}

pub fn cleanup_stale_vms(socket_dir: &Path) {
    let Ok(entries) = std::fs::read_dir(socket_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(file_name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !file_name.starts_with("fc-") || !file_name.ends_with(".meta") {
            continue;
        }
        cleanup_stale_vm(&path);
    }
}

fn cleanup_stale_vm(meta_path: &Path) {
    let Ok(content) = std::fs::read_to_string(meta_path) else {
        return;
    };
    let lines: Vec<&str> = content.lines().collect();
    if lines.len() >= 3 {
        if let Ok(pid) = lines[0].trim().parse::<i32>() {
            let _ = kill(Pid::from_raw(pid), Signal::SIGTERM);
        }
        delete_tap(lines[1].trim());
        let _ = std::fs::remove_file(lines[2].trim());
    }
    let _ = std::fs::remove_file(meta_path.with_extension("socket"));
    let _ = std::fs::remove_file(meta_path);
}

fn write_vm_meta(meta_path: &Path, pid: u32, tap_name: &str, rootfs_copy: &Path) {
    let content = format!("{pid}\n{tap_name}\n{}", rootfs_copy.display());
    let _ = std::fs::write(meta_path, content);
}

async fn copy_rootfs(src: &Path, dst: &Path) -> Result<()> {
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

fn spawn_firecracker(socket_path: &Path) -> Result<Child> {
    Ok(Command::new("firecracker")
        .args(["--api-sock", &socket_path.to_string_lossy()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .kill_on_drop(false)
        .process_group(0)
        .spawn()?)
}

async fn check_still_running(child: &mut Child) -> Result<()> {
    tokio::time::sleep(Duration::from_millis(500)).await;
    match child.try_wait()? {
        Some(status) => bail!("firecracker exited immediately after start: {status}"),
        None => Ok(()),
    }
}

async fn wait_for_socket(socket_path: &Path) -> Result<()> {
    for _ in 0..50 {
        if socket_path.exists() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    bail!("timed out waiting for firecracker socket")
}

async fn configure_vm(
    socket_path: &Path,
    rootfs_copy: &Path,
    vm_config: &VmConfig,
    tap_name: &str,
    mac: &str,
    boot_args: &str,
) -> Result<()> {
    configure_machine_config(socket_path, vm_config).await?;
    configure_boot_source(socket_path, vm_config, boot_args).await?;
    configure_rootfs_drive(socket_path, rootfs_copy).await?;
    configure_network_interface(socket_path, tap_name, mac).await?;
    if let Some(metadata) = &vm_config.mmds_metadata {
        configure_mmds(socket_path, vm_config, metadata).await?;
    }
    Ok(())
}

async fn configure_machine_config(socket_path: &Path, vm_config: &VmConfig) -> Result<()> {
    set_machine_config(
        socket_path,
        &MachineConfig {
            vcpu_count: vm_config.vcpu_count,
            mem_size_mib: vm_config.mem_size_mib,
        },
    )
    .await?;
    Ok(())
}

async fn configure_boot_source(
    socket_path: &Path,
    vm_config: &VmConfig,
    boot_args: &str,
) -> Result<()> {
    set_boot_source(
        socket_path,
        &BootSource {
            kernel_image_path: vm_config.kernel_path.to_string_lossy().into_owned(),
            boot_args: boot_args.to_string(),
        },
    )
    .await?;
    Ok(())
}

async fn configure_rootfs_drive(socket_path: &Path, rootfs_copy: &Path) -> Result<()> {
    set_drive(
        socket_path,
        &Drive {
            drive_id: "rootfs".to_string(),
            path_on_host: rootfs_copy.to_string_lossy().into_owned(),
            is_root_device: true,
            is_read_only: false,
        },
    )
    .await?;
    Ok(())
}

async fn configure_network_interface(socket_path: &Path, tap_name: &str, mac: &str) -> Result<()> {
    set_network_interface(
        socket_path,
        &NetworkInterface {
            iface_id: "net1".to_string(),
            guest_mac: mac.to_string(),
            host_dev_name: tap_name.to_string(),
        },
    )
    .await?;
    Ok(())
}

async fn configure_mmds(
    socket_path: &Path,
    vm_config: &VmConfig,
    metadata: &serde_json::Value,
) -> Result<()> {
    let mmds_config = if vm_config.mmds_imds_compat {
        imds_compat_mmds_config(vec!["net1".to_string()])
    } else {
        MmdsConfig {
            version: None,
            network_interfaces: vec!["net1".to_string()],
            ipv4_address: None,
            imds_compat: None,
        }
    };
    set_mmds_config(socket_path, &mmds_config).await?;
    put_mmds(socket_path, metadata).await?;
    Ok(())
}

fn delete_tap(tap_name: &str) {
    let _ = std::process::Command::new(resolve_net_helper_path())
        .args(["tap-delete", tap_name])
        .status();
}

async fn create_tap(tap_name: &str, tap_ip: &str) -> Result<()> {
    let status = Command::new(resolve_net_helper_path())
        .args(["tap-create", tap_name, tap_ip])
        .status()
        .await?;
    if !status.success() {
        bail!(
            "net-helper tap-create failed for {tap_name}: exit {}",
            status.code().unwrap_or(-1)
        );
    }
    Ok(())
}

fn resolve_net_helper_path() -> String {
    std::env::var("NET_HELPER_PATH").unwrap_or_else(|_| "/usr/local/bin/net-helper".to_string())
}

fn format_tap_name(idx: u32) -> String {
    format!("tap{idx}")
}
fn format_tap_ip(idx: u32) -> String {
    format!("172.16.{idx}.1/30")
}
fn format_guest_ip(idx: u32) -> String {
    format!("172.16.{idx}.2")
}
fn format_guest_mac(idx: u32) -> String {
    format!("06:00:AC:10:{:02X}:02", idx)
}

async fn fetch_host_iface_name() -> Option<String> {
    let output = Command::new("ip")
        .args(["route", "list", "default"])
        .output()
        .await
        .ok()?;
    let route_output = String::from_utf8_lossy(&output.stdout);
    let mut tokens = route_output.split_whitespace();
    while let Some(token) = tokens.next() {
        if token == "dev" {
            return tokens.next().map(|s| s.to_string());
        }
    }
    None
}

pub async fn setup_host_networking() {
    let Some(host_iface) = fetch_host_iface_name().await else {
        warn!("could not determine host interface, skipping NAT setup");
        return;
    };
    match Command::new(resolve_net_helper_path())
        .args(["setup-nat", &host_iface])
        .status()
        .await
    {
        Ok(s) if s.success() => {}
        Ok(s) => warn!(
            "net-helper setup-nat failed: exit {}",
            s.code().unwrap_or(-1)
        ),
        Err(e) => warn!("failed to run net-helper setup-nat: {e}"),
    }
}
