use std::path::Path;
use tokio::{fs, process::Command};

use crate::network::delete_tap;

pub async fn cleanup_stale_vms(net_helper_path: &Path, jailer_chroot_base: &Path) {
    kill_stale_firecracker_processes().await;
    delete_stale_tap_interfaces(net_helper_path).await;
    delete_stale_chroot_dirs(jailer_chroot_base).await;
}

async fn kill_stale_firecracker_processes() {
    let _ = Command::new("pkill").args(["-f", "firecracker"]).status().await;
}

async fn delete_stale_tap_interfaces(net_helper_path: &Path) {
    let Ok(output) = Command::new("ip")
        .args(["link", "show", "type", "tun"])
        .output()
        .await
    else {
        return;
    };
    let output = String::from_utf8_lossy(&output.stdout);
    for line in output.lines() {
        let Some(name) = parse_tap_interface_name(line) else {
            continue;
        };
        delete_tap(net_helper_path, name).await;
    }
}

fn parse_tap_interface_name(line: &str) -> Option<&str> {
    // lines look like: "5: tap0: <...> ..."
    let name = line.split(':').nth(1)?.trim();
    name.starts_with("tap").then_some(name)
}

async fn delete_stale_chroot_dirs(chroot_base: &Path) {
    let firecracker_dir = chroot_base.join("firecracker");
    let Ok(mut entries) = fs::read_dir(&firecracker_dir).await else {
        return;
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        let _ = fs::remove_dir_all(entry.path()).await;
    }
}
