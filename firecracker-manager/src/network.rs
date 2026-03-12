use anyhow::{bail, Context, Result};
use std::path::Path;
use tokio::process::Command;
use tracing::warn;

pub(crate) async fn create_tap(net_helper_path: &Path, tap_name: &str, tap_ip: &str) -> Result<()> {
    let status = Command::new(net_helper_path)
        .args(["tap-create", tap_name, tap_ip])
        .status()
        .await?;
    if !status.success() {
        bail!(
            "net-helper tap-create failed for {tap_name}: {status}"
        );
    }
    Ok(())
}

pub(crate) async fn delete_tap(net_helper_path: &Path, tap_name: &str) {
    let _ = Command::new(net_helper_path)
        .args(["tap-delete", tap_name])
        .status()
        .await;
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

pub async fn setup_host_networking(net_helper_path: &Path) {
    let Some(host_iface) = fetch_host_iface_name().await else {
        warn!("could not determine host interface, skipping NAT setup");
        return;
    };
    run_nat_setup(net_helper_path, &host_iface)
        .await
        .unwrap_or_else(|e| warn!("{e}"));
}

async fn run_nat_setup(net_helper_path: &Path, host_iface: &str) -> Result<()> {
    let status = Command::new(net_helper_path)
        .args(["setup-nat", host_iface])
        .status()
        .await
        .context("failed to run net-helper setup-nat")?;
    if !status.success() {
        bail!("net-helper setup-nat failed: {status}");
    }
    Ok(())
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
