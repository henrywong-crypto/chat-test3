use nix::{
    sys::signal::{kill, Signal},
    unistd::Pid,
};
use std::path::Path;

use crate::network::delete_tap;

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
