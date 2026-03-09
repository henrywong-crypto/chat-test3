use std::path::Path;

use crate::network::delete_tap;

pub fn cleanup_stale_vms(socket_dir: &Path, net_helper_path: &Path) {
    kill_stale_firecracker_processes();
    delete_stale_socket_dir_files(socket_dir);
    delete_stale_tap_interfaces(net_helper_path);
}

fn kill_stale_firecracker_processes() {
    let _ = std::process::Command::new("pkill")
        .args(["-f", "firecracker"])
        .status();
}

fn delete_stale_socket_dir_files(socket_dir: &Path) {
    let Ok(entries) = std::fs::read_dir(socket_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if name.starts_with("fc-") && name.ends_with(".socket")
        {
            let _ = std::fs::remove_file(&path);
        }
    }
}

fn delete_stale_tap_interfaces(net_helper_path: &Path) {
    let Ok(output) = std::process::Command::new("ip")
        .args(["link", "show", "type", "tun"])
        .output()
    else {
        return;
    };
    let output = String::from_utf8_lossy(&output.stdout);
    for line in output.lines() {
        let Some(name) = parse_tap_interface_name(line) else {
            continue;
        };
        delete_tap(net_helper_path, name);
    }
}

fn parse_tap_interface_name(line: &str) -> Option<&str> {
    // lines look like: "5: tap0: <...> ..."
    let name = line.split(':').nth(1)?.trim();
    name.starts_with("tap").then_some(name)
}
