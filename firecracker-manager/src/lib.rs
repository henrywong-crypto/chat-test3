mod mmds_iam;
pub use mmds_iam::{build_mmds_iam_refresh_patch, build_mmds_with_iam, imds_compat_mmds_config, system_time_to_iso8601, ImdsCredential};

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use firecracker_client::{
    get_network_interfaces, put_mmds, set_boot_source, set_drive, set_machine_config,
    set_mmds_config, set_network_interface, start_instance, BootSource, Drive, MachineConfig,
    MmdsConfig, NetworkInterface,
};
use thiserror::Error;
use tokio::process::Command;

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Firecracker(#[from] firecracker_client::Error),
    #[error("network setup failed: {0}")]
    NetworkSetup(String),
}

pub type Result<T> = std::result::Result<T, Error>;

static VM_NET_COUNTER: AtomicU32 = AtomicU32::new(0);

pub struct VmConfig {
    pub id: String,
    pub socket_path: PathBuf,
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
    tap_name: String,
}

pub struct VmGuard {
    pub id: String,
    pub guest_ip: String,
    pub socket_path: PathBuf,
    tap_name: String,
}

impl VmGuard {
    /// Remove this VM's TAP interface. Call on explicit deletion.
    /// Drop is a no-op so the TAP survives server restarts.
    pub fn delete(self) {
        delete_tap(&self.tap_name);
    }
}

impl Drop for VmGuard {
    fn drop(&mut self) {}
}

impl Vm {
    pub fn into_guard(self) -> VmGuard {
        VmGuard {
            id: self.id,
            guest_ip: self.guest_ip,
            socket_path: self.socket_path,
            tap_name: self.tap_name,
        }
    }
}

/// Scan `socket_dir` for `fc-*.socket` files, query each via the Firecracker API,
/// and return live VMs. Stale sockets (no response) are removed.
pub async fn reconcile_vms(socket_dir: &Path) -> Vec<(String, VmGuard)> {
    let mut recovered = Vec::new();
    let entries = match std::fs::read_dir(socket_dir) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("reconcile: failed to read {:?}: {e}", socket_dir);
            return recovered;
        }
    };
    for entry in entries.flatten() {
        let socket_path = entry.path();
        let fname = match socket_path.file_name().and_then(|n| n.to_str()) {
            Some(f) => f.to_string(),
            None => continue,
        };
        if !fname.starts_with("fc-") || !fname.ends_with(".socket") {
            continue;
        }
        let vm_id = fname
            .trim_start_matches("fc-")
            .trim_end_matches(".socket")
            .to_string();

        let ifaces = match get_network_interfaces(&socket_path).await {
            Ok(i) => i,
            Err(_) => {
                eprintln!("reconcile: {fname} not responding, removing stale socket");
                let _ = std::fs::remove_file(&socket_path);
                continue;
            }
        };
        let tap_name = match ifaces.into_iter().find(|i| i.iface_id == "net1") {
            Some(i) => i.host_dev_name,
            None => {
                eprintln!("reconcile: no net1 interface for {vm_id}, skipping");
                continue;
            }
        };
        let net_idx: u32 = match tap_name.trim_start_matches("tap").parse() {
            Ok(n) => n,
            Err(_) => {
                eprintln!("reconcile: cannot parse tap index from {tap_name}, skipping");
                continue;
            }
        };
        let guest_ip = vm_guest_ip(net_idx);
        eprintln!("reconcile: recovered vm {vm_id} guest_ip={guest_ip}");

        recovered.push((vm_id.clone(), VmGuard {
            id: vm_id,
            guest_ip,
            socket_path,
            tap_name,
        }));
    }
    recovered
}

pub async fn create_vm(vm_config: &VmConfig) -> Result<Vm> {
    let net_idx = VM_NET_COUNTER.fetch_add(1, Ordering::Relaxed) % 254;
    let tap_name = vm_tap_name(net_idx);
    let tap_ip = vm_tap_ip(net_idx);
    let mac = vm_guest_mac(net_idx);
    let guest_ip = vm_guest_ip(net_idx);

    let boot_args = format!(
        "{} ip={guest_ip}::172.16.{net_idx}.1:255.255.255.252::eth0:none",
        vm_config.boot_args
    );

    create_tap(&tap_name, &tap_ip).await?;
    configure_vm(&vm_config.socket_path, vm_config, &tap_name, &mac, &boot_args).await?;
    start_instance(&vm_config.socket_path).await?;

    Ok(Vm { id: vm_config.id.clone(), guest_ip, socket_path: vm_config.socket_path.clone(), tap_name })
}


async fn configure_vm(
    socket_path: &Path,
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

fn delete_tap(tap_name: &str) {
    let helper = net_helper_path();
    let _ = std::process::Command::new(&helper)
        .args(["tap-delete", tap_name])
        .status();
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
        eprintln!("warning: could not determine host interface, skipping NAT setup");
        return;
    };
    let helper = net_helper_path();
    match Command::new(&helper).args(["setup-nat", &host_iface]).status().await {
        Ok(s) if s.success() => {}
        Ok(s) => eprintln!("warning: net-helper setup-nat failed: exit {}", s.code().unwrap_or(-1)),
        Err(e) => eprintln!("warning: failed to run net-helper setup-nat: {e}"),
    }
}
