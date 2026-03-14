use anyhow::{Context, Result, bail};
use std::path::Path;
use tokio::process::Command;
use tracing::warn;

pub(crate) async fn create_tap(net_helper_path: &Path, tap_name: &str, tap_ip: &str) -> Result<()> {
    let status = Command::new(net_helper_path)
        .args(["tap-create", tap_name, tap_ip])
        .status()
        .await?;
    if !status.success() {
        bail!("net-helper tap-create failed for {tap_name}: {status}");
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

#[cfg(test)]
mod tests {
    use super::*;

    // ── format_tap_name ───────────────────────────────────────────────────────

    #[test]
    fn test_format_tap_name_single_digit() {
        assert_eq!(format_tap_name(0), "tap0");
        assert_eq!(format_tap_name(9), "tap9");
    }

    #[test]
    fn test_format_tap_name_multi_digit() {
        assert_eq!(format_tap_name(10), "tap10");
        assert_eq!(format_tap_name(253), "tap253");
    }

    // ── format_tap_ip ─────────────────────────────────────────────────────────

    #[test]
    fn test_format_tap_ip_structure() {
        assert_eq!(format_tap_ip(0), "172.16.0.1/30");
        assert_eq!(format_tap_ip(1), "172.16.1.1/30");
        assert_eq!(format_tap_ip(255), "172.16.255.1/30");
    }

    // ── format_guest_ip ───────────────────────────────────────────────────────

    #[test]
    fn test_format_guest_ip_structure() {
        assert_eq!(format_guest_ip(0), "172.16.0.2");
        assert_eq!(format_guest_ip(1), "172.16.1.2");
        assert_eq!(format_guest_ip(255), "172.16.255.2");
    }

    #[test]
    fn test_tap_and_guest_ip_share_same_subnet_for_same_idx() {
        // For each idx, tap (.1) and guest (.2) are in the same /30 block.
        for idx in [0u32, 1, 128, 253] {
            let tap_ip = format_tap_ip(idx);
            let guest_ip = format_guest_ip(idx);
            let tap_prefix = tap_ip.trim_end_matches(".1/30");
            let guest_prefix = guest_ip.trim_end_matches(".2");
            assert_eq!(tap_prefix, guest_prefix, "idx={idx}: subnet mismatch");
        }
    }

    // ── format_guest_mac ─────────────────────────────────────────────────────

    #[test]
    fn test_format_guest_mac_zero_padded_for_low_idx() {
        assert_eq!(format_guest_mac(0), "06:00:AC:10:00:02");
        assert_eq!(format_guest_mac(1), "06:00:AC:10:01:02");
        assert_eq!(format_guest_mac(15), "06:00:AC:10:0F:02");
    }

    #[test]
    fn test_format_guest_mac_two_hex_digits_for_high_idx() {
        assert_eq!(format_guest_mac(16), "06:00:AC:10:10:02");
        assert_eq!(format_guest_mac(255), "06:00:AC:10:FF:02");
    }

    #[test]
    fn test_format_guest_mac_uses_uppercase_hex() {
        let mac = format_guest_mac(0xAB);
        assert!(mac.contains("AB"), "expected uppercase hex in {mac}");
    }
}
