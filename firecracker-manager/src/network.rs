use anyhow::{bail, Result};
use tokio::process::Command;
use tracing::warn;

pub(crate) async fn create_tap(tap_name: &str, tap_ip: &str) -> Result<()> {
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

pub(crate) fn delete_tap(tap_name: &str) {
    let _ = std::process::Command::new(resolve_net_helper_path())
        .args(["tap-delete", tap_name])
        .status();
}

pub(crate) fn format_tap_name(idx: u32) -> String {
    format!("tap{idx}")
}

pub(crate) fn format_tap_ip(idx: u32) -> String {
    format!("172.16.{idx}.1/30")
}

pub(crate) fn format_guest_ip(idx: u32) -> String {
    format!("172.16.{idx}.2")
}

pub(crate) fn format_guest_mac(idx: u32) -> String {
    format!("06:00:AC:10:{:02X}:02", idx)
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

fn resolve_net_helper_path() -> String {
    std::env::var("NET_HELPER_PATH").unwrap_or_else(|_| "/usr/local/bin/net-helper".to_string())
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
