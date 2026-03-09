mod mmds_iam;
pub use mmds_iam::{build_mmds_iam_refresh_patch, build_mmds_with_iam, imds_compat_mmds_config, system_time_to_iso8601, ImdsCredential};

use std::io;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use firecracker_client::{
    put_mmds, set_boot_source, set_drive, set_machine_config,
    set_mmds_config, set_network_interface, start_instance, BootSource, Drive, MachineConfig,
    MmdsConfig, NetworkInterface,
};
use nix::sys::signal::{kill, Signal};
use nix::unistd::Pid;
use thiserror::Error;
use tracing::warn;
use tokio::process::{Child, Command};

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Nix(#[from] nix::Error),
    #[error(transparent)]
    Firecracker(#[from] firecracker_client::Error),
    #[error("timed out waiting for firecracker socket")]
    SocketTimeout,
    #[error("process exited before pid was available")]
    ProcessExited,
    #[error("firecracker exited immediately after start: {0}")]
    FirecrackerExited(String),
    #[error("network setup failed: {0}")]
    NetworkSetup(String),
}

pub type Result<T> = std::result::Result<T, Error>;

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
        // drop runs the cleanup
    }
}

impl Drop for VmGuard {
    fn drop(&mut self) {
        let _ = kill(Pid::from_raw(self.pid as i32), Signal::SIGTERM);
        delete_tap(&self.tap_name);
        let _ = std::fs::remove_file(&self.rootfs_copy);
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
    let tap_name = vm_tap_name(net_idx);
    let tap_ip = vm_tap_ip(net_idx);
    let mac = vm_guest_mac(net_idx);
    let guest_ip = vm_guest_ip(net_idx);
    let socket_path = vm_config.socket_dir.join(format!("fc-{}.socket", vm_config.id));
    let rootfs_copy = vm_config.socket_dir.join(format!("rootfs-{}.ext4", vm_config.id));
    let meta_path = vm_config.socket_dir.join(format!("fc-{}.meta", vm_config.id));

    let boot_args = format!(
        "{} ip={guest_ip}::172.16.{net_idx}.1:255.255.255.252::eth0:none",
        vm_config.boot_args
    );

    create_tap(&tap_name, &tap_ip).await?;
    copy_rootfs(&vm_config.rootfs_path, &rootfs_copy).await?;
    let mut child = spawn_firecracker(&socket_path)?;
    let pid = child.id().ok_or(Error::ProcessExited)?;
    write_vm_meta(&meta_path, pid, &tap_name, &rootfs_copy);
    wait_for_socket(&socket_path).await?;
    configure_vm(&socket_path, &rootfs_copy, vm_config, &tap_name, &mac, &boot_args).await?;
    start_instance(&socket_path).await?;
    check_still_running(&mut child).await?;

    Ok(Vm { id: vm_config.id.clone(), guest_ip, socket_path, pid, _child: child, tap_name, rootfs_copy, meta_path })
}

/// Kill and clean up any VMs left over from a previous server run.
/// Call this once on startup before accepting requests.
pub fn cleanup_stale_vms(socket_dir: &Path) {
    let Ok(entries) = std::fs::read_dir(socket_dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(file_name) = path.file_name().and_then(|n| n.to_str()) else { continue };
        if !file_name.starts_with("fc-") || !file_name.ends_with(".meta") { continue }
        cleanup_stale_vm(&path);
    }
}

fn cleanup_stale_vm(meta_path: &Path) {
    let Ok(content) = std::fs::read_to_string(meta_path) else { return };
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
        .args(["--sparse=always", &src.to_string_lossy(), &dst.to_string_lossy()])
        .status()
        .await?;
    if !status.success() {
        return Err(Error::NetworkSetup(format!(
            "failed to copy rootfs from {} to {}",
            src.display(), dst.display()
        )));
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
    match child.try_wait() {
        Ok(Some(status)) => Err(Error::FirecrackerExited(status.to_string())),
        Ok(None) => Ok(()),
        Err(e) => Err(Error::Io(e)),
    }
}

async fn wait_for_socket(socket_path: &Path) -> Result<()> {
    for _ in 0..50 {
        if socket_path.exists() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err(Error::SocketTimeout)
}

async fn configure_vm(
    socket_path: &Path,
    rootfs_copy: &Path,
    vm_config: &VmConfig,
    tap_name: &str,
    mac: &str,
    boot_args: &str,
) -> Result<()> {
    set_machine_config(socket_path, &MachineConfig {
        vcpu_count: vm_config.vcpu_count,
        mem_size_mib: vm_config.mem_size_mib,
    })
    .await?;
    set_boot_source(socket_path, &BootSource {
        kernel_image_path: vm_config.kernel_path.to_string_lossy().into_owned(),
        boot_args: boot_args.to_string(),
    })
    .await?;
    set_drive(socket_path, &Drive {
        drive_id: "rootfs".to_string(),
        path_on_host: rootfs_copy.to_string_lossy().into_owned(),
        is_root_device: true,
        is_read_only: false,
    })
    .await?;
    set_network_interface(socket_path, &NetworkInterface {
        iface_id: "net1".to_string(),
        guest_mac: mac.to_string(),
        host_dev_name: tap_name.to_string(),
    })
    .await?;
    if let Some(metadata) = &vm_config.mmds_metadata {
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
    }
    Ok(())
}

fn delete_tap(tap_name: &str) {
    let _ = std::process::Command::new(net_helper_path())
        .args(["tap-delete", tap_name])
        .status();
}

async fn create_tap(tap_name: &str, tap_ip: &str) -> Result<()> {
    let status = Command::new(net_helper_path())
        .args(["tap-create", tap_name, tap_ip])
        .status()
        .await?;
    if !status.success() {
        return Err(Error::NetworkSetup(format!(
            "net-helper tap-create failed for {tap_name}: exit {}",
            status.code().unwrap_or(-1)
        )));
    }
    Ok(())
}

fn net_helper_path() -> String {
    std::env::var("NET_HELPER_PATH").unwrap_or_else(|_| "/usr/local/bin/net-helper".to_string())
}

fn vm_tap_name(idx: u32) -> String { format!("tap{idx}") }
fn vm_tap_ip(idx: u32) -> String   { format!("172.16.{idx}.1/30") }
fn vm_guest_ip(idx: u32) -> String  { format!("172.16.{idx}.2") }
fn vm_guest_mac(idx: u32) -> String { format!("06:00:AC:10:{:02X}:02", idx) }

async fn get_host_iface() -> Option<String> {
    let output = Command::new("ip").args(["route", "list", "default"]).output().await.ok()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut tokens = stdout.split_whitespace();
    while let Some(token) = tokens.next() {
        if token == "dev" {
            return tokens.next().map(|s| s.to_string());
        }
    }
    None
}

pub async fn setup_host_networking() {
    let Some(host_iface) = get_host_iface().await else {
        warn!("could not determine host interface, skipping NAT setup");
        return;
    };
    match Command::new(net_helper_path()).args(["setup-nat", &host_iface]).status().await {
        Ok(s) if s.success() => {}
        Ok(s) => warn!("net-helper setup-nat failed: exit {}", s.code().unwrap_or(-1)),
        Err(e) => warn!("failed to run net-helper setup-nat: {e}"),
    }
}
