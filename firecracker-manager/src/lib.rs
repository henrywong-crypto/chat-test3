use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use firecracker_client::{
    set_boot_source, set_drive, set_machine_config, set_network_interface, start_instance,
    BootSource, Drive, MachineConfig, NetworkInterface,
};
use nix::sys::signal::{kill, Signal};
use nix::unistd::{dup, Pid};
use terminal_bridge::{open_pty, PtyMaster, PtySlave};
use thiserror::Error;
use tokio::process::{Child, Command};

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Nix(#[from] nix::Error),
    #[error(transparent)]
    Pty(#[from] terminal_bridge::Error),
    #[error(transparent)]
    Firecracker(#[from] firecracker_client::Error),
    #[error("timed out waiting for firecracker socket")]
    SocketTimeout,
    #[error("process exited before pid was available")]
    ProcessExited,
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
}

pub struct Vm {
    pub id: String,
    pub socket_path: PathBuf,
    pub pid: u32,
    pub pty_master: PtyMaster,
    child: Child,
    tap_name: String,
}

pub struct VmGuard {
    pub id: String,
    pub socket_path: PathBuf,
    pub pid: u32,
    _child: Child,
    tap_name: String,
}

impl Drop for VmGuard {
    fn drop(&mut self) {
        let _ = std::process::Command::new("ip")
            .args(["link", "del", &self.tap_name])
            .status();
    }
}

impl Vm {
    pub fn into_pty_master(self) -> (PtyMaster, VmGuard) {
        let guard = VmGuard {
            id: self.id,
            socket_path: self.socket_path,
            pid: self.pid,
            _child: self.child,
            tap_name: self.tap_name,
        };
        (self.pty_master, guard)
    }
}

pub async fn create_vm(vm_config: &VmConfig) -> Result<Vm> {
    let net_idx = VM_NET_COUNTER.fetch_add(1, Ordering::Relaxed) % 254;
    let tap_name = vm_tap_name(net_idx);
    let tap_ip = vm_tap_ip(net_idx);
    let mac = vm_guest_mac(net_idx);

    let pty_pair = open_pty()?;
    let socket_path = build_socket_path(&vm_config.socket_dir, &vm_config.id);
    create_tap(&tap_name, &tap_ip).await?;
    let child = spawn_firecracker(&socket_path, pty_pair.slave)?;
    let pid = child.id().ok_or(Error::ProcessExited)?;
    wait_for_socket(&socket_path).await?;
    configure_vm(&socket_path, vm_config, &tap_name, &mac).await?;
    start_instance(&socket_path).await?;
    Ok(Vm { id: vm_config.id.clone(), socket_path, pid, pty_master: pty_pair.master, child, tap_name })
}

pub fn kill_vm(vm_pid: u32) -> Result<()> {
    kill(Pid::from_raw(vm_pid as i32), Signal::SIGTERM)?;
    Ok(())
}

pub fn check_vm_alive(vm_pid: u32) -> bool {
    kill(Pid::from_raw(vm_pid as i32), None).is_ok()
}

fn build_socket_path(socket_dir: &Path, vm_id: &str) -> PathBuf {
    socket_dir.join(format!("fc-{vm_id}.socket"))
}

fn spawn_firecracker(socket_path: &Path, slave: PtySlave) -> Result<Child> {
    let slave_fd = slave.into_owned_fd();
    let stdout_fd = dup_fd(&slave_fd)?;
    let stderr_fd = dup_fd(&slave_fd)?;
    let child = Command::new("firecracker")
        .args(["--api-sock", &socket_path.to_string_lossy()])
        .stdin(Stdio::from(slave_fd))
        .stdout(Stdio::from(stdout_fd))
        .stderr(Stdio::from(stderr_fd))
        .kill_on_drop(true)
        .spawn()?;
    Ok(child)
}

async fn configure_vm(socket_path: &Path, vm_config: &VmConfig, tap_name: &str, mac: &str) -> Result<()> {
    set_machine_config(socket_path, &MachineConfig {
        vcpu_count: vm_config.vcpu_count,
        mem_size_mib: vm_config.mem_size_mib,
    })
    .await?;
    set_boot_source(socket_path, &BootSource {
        kernel_image_path: vm_config.kernel_path.to_string_lossy().into_owned(),
        boot_args: vm_config.boot_args.clone(),
    })
    .await?;
    set_drive(socket_path, &Drive {
        drive_id: "rootfs".to_string(),
        path_on_host: vm_config.rootfs_path.to_string_lossy().into_owned(),
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
    Ok(())
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

fn vm_tap_name(idx: u32) -> String {
    format!("tap{idx}")
}

fn vm_tap_ip(idx: u32) -> String {
    format!("172.16.{idx}.1/30")
}

fn vm_guest_mac(idx: u32) -> String {
    format!("06:00:AC:10:{:02X}:02", idx)
}

async fn create_tap(tap_name: &str, tap_ip: &str) -> Result<()> {
    let _ = Command::new("ip").args(["link", "del", tap_name]).status().await;
    run_ip(&["tuntap", "add", "dev", tap_name, "mode", "tap"]).await?;
    run_ip(&["addr", "add", tap_ip, "dev", tap_name]).await?;
    run_ip(&["link", "set", "dev", tap_name, "up"]).await?;
    Ok(())
}

async fn run_ip(args: &[&str]) -> Result<()> {
    let status = Command::new("ip").args(args).status().await?;
    if !status.success() {
        return Err(Error::NetworkSetup(format!("ip {:?} failed", args)));
    }
    Ok(())
}

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
    let _ = tokio::fs::write("/proc/sys/net/ipv4/ip_forward", "1").await;
    let _ = Command::new("iptables").args(["-P", "FORWARD", "ACCEPT"]).status().await;
    let Some(host_iface) = get_host_iface().await else { return };
    // Best-effort delete to avoid duplicates on restart, then add (matches `|| true` in reference script)
    let _ = Command::new("iptables")
        .args(["-t", "nat", "-D", "POSTROUTING", "-o", &host_iface, "-j", "MASQUERADE"])
        .stderr(Stdio::null())
        .status()
        .await;
    let _ = Command::new("iptables")
        .args(["-t", "nat", "-A", "POSTROUTING", "-o", &host_iface, "-j", "MASQUERADE"])
        .status()
        .await;
}

fn dup_fd(fd: &OwnedFd) -> Result<OwnedFd> {
    let raw_fd = dup(fd.as_raw_fd())?;
    Ok(unsafe { OwnedFd::from_raw_fd(raw_fd) })
}
