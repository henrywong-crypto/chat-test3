mod mmds_iam;
pub use mmds_iam::{build_mmds_iam_refresh_patch, build_mmds_with_iam, imds_compat_mmds_config, system_time_to_iso8601, ImdsCredential};

use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use firecracker_client::{
    put_mmds, set_boot_source, set_drive, set_machine_config, set_mmds_config, set_network_interface,
    start_instance, BootSource, Drive, MachineConfig, MmdsConfig, NetworkInterface,
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
    /// MMDS payload (e.g. from [build_mmds_with_iam]). If [mmds_imds_compat] is true, use
    /// [imds_compat_mmds_config] so the guest can use the AWS default credential chain.
    pub mmds_metadata: Option<serde_json::Value>,
    /// When true, MMDS is configured with imds_compat and V2 so EC2 IMDS–style credential
    /// paths work (required when [mmds_metadata] contains IAM credentials for the SDK).
    pub mmds_imds_compat: bool,
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
        let helper = net_helper_path();
        let _ = std::process::Command::new(&helper)
            .args(["tap-delete", &self.tap_name])
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
    let helper = net_helper_path();
    let status = Command::new(&helper)
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
        eprintln!("warning: could not determine host interface, skipping NAT setup");
        return;
    };
    let helper = net_helper_path();
    match Command::new(&helper).args(["setup-nat", &host_iface]).status().await {
        Ok(s) if s.success() => {}
        Ok(s) => eprintln!(
            "warning: net-helper setup-nat failed: exit {}",
            s.code().unwrap_or(-1)
        ),
        Err(e) => eprintln!("warning: failed to run net-helper setup-nat: {e}"),
    }
}

fn dup_fd(fd: &OwnedFd) -> Result<OwnedFd> {
    let raw_fd = dup(fd.as_raw_fd())?;
    Ok(unsafe { OwnedFd::from_raw_fd(raw_fd) })
}
