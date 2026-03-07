//! Helpers for passing IAM role credentials to Firecracker guests via MMDS in
//! EC2 IMDS-compatible format so the AWS SDK (e.g. Rust default credential chain) works.
//!
//! See [docs/mmds-iam-role.md](../../docs/mmds-iam-role.md) for the full design.

use serde::Serialize;

/// EC2 IMDS credential response shape. The guest (e.g. AWS SDK) expects this
/// exact structure at `latest/meta-data/iam/security-credentials/<role-name>`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct ImdsCredential {
    pub code: String,
    pub last_updated: String,
    #[serde(rename = "Type")]
    pub type_: String,
    pub access_key_id: String,
    pub secret_access_key: String,
    pub token: String,
    pub expiration: String,
}

impl ImdsCredential {
    /// Build from temporary credential fields (e.g. from STS AssumeRole / GetSessionToken).
    pub fn new(
        access_key_id: impl Into<String>,
        secret_access_key: impl Into<String>,
        token: impl Into<String>,
        expiration: impl Into<String>,
    ) -> Self {
        Self {
            code: "Success".to_string(),
            last_updated: expiration_iso8601_now(),
            type_: "AWS-HMAC".to_string(),
            access_key_id: access_key_id.into(),
            secret_access_key: secret_access_key.into(),
            token: token.into(),
            expiration: expiration.into(),
        }
    }
}

fn expiration_iso8601_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    format_iso8601(secs)
}

pub fn system_time_to_iso8601(t: std::time::SystemTime) -> String {
    let secs = t
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format_iso8601(secs)
}

fn format_iso8601(secs: u64) -> String {
    let (year, month, day, h, m, s) = secs_to_ymd_hms(secs);
    format!("{year:04}-{month:02}-{day:02}T{h:02}:{m:02}:{s:02}Z")
}

fn secs_to_ymd_hms(secs: u64) -> (u64, u64, u64, u64, u64, u64) {
    const SECS_PER_DAY: u64 = 86400;
    let days = secs / SECS_PER_DAY;
    let rem = secs % SECS_PER_DAY;
    let h = rem / 3600;
    let m = (rem % 3600) / 60;
    let s = rem % 60;
    let (y, m_d) = days_to_year(days);
    let (mo, d) = day_of_year_to_md(m_d, is_leap(y));
    (y, mo, d, h, m, s)
}

fn days_to_year(days: u64) -> (u64, u64) {
    let mut y = 1970u64;
    let mut d = days;
    loop {
        let len = if is_leap(y) { 366 } else { 365 };
        if d < len {
            return (y, d);
        }
        d -= len;
        y += 1;
    }
}

fn is_leap(y: u64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || (y % 400 == 0)
}

fn day_of_year_to_md(doy: u64, leap: bool) -> (u64, u64) {
    let lens: [u64; 12] = if leap {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };
    let mut d = doy;
    for (i, &len) in lens.iter().enumerate() {
        if d < len {
            return ((i + 1) as u64, d + 1);
        }
        d -= len;
    }
    (12, 31)
}

/// Build full MMDS JSON for initial `PUT /mmds`: instance-id + IAM credentials
/// in EC2 IMDS shape. Use with `imds_compat: true` and `version: "V2"` in MmdsConfig.
pub fn build_mmds_with_iam(
    instance_id: &str,
    role_name: &str,
    credential: &ImdsCredential,
) -> serde_json::Value {
    let cred_value = serde_json::to_value(credential).unwrap();
    let mut security_credentials = serde_json::Map::new();
    security_credentials.insert(role_name.to_string(), cred_value);
    serde_json::json!({
        "latest": {
            "meta-data": {
                "instance-id": instance_id,
                "iam": {
                    "security-credentials": security_credentials,
                }
            }
        }
    })
}

/// Build MMDS PATCH payload to refresh only the IAM credentials. Merge this
/// with existing MMDS via `PATCH /mmds` so the rest of the metadata is unchanged.
pub fn build_mmds_iam_refresh_patch(role_name: &str, credential: &ImdsCredential) -> serde_json::Value {
    let cred_value = serde_json::to_value(credential).unwrap();
    let mut security_credentials = serde_json::Map::new();
    security_credentials.insert(role_name.to_string(), cred_value);
    serde_json::json!({
        "latest": {
            "meta-data": {
                "iam": {
                    "security-credentials": security_credentials,
                }
            }
        }
    })
}

/// MmdsConfig tuned for EC2 IMDS compatibility so the AWS SDK default credential
/// chain in the guest works without env vars.
pub fn imds_compat_mmds_config(network_interface_ids: Vec<String>) -> firecracker_client::MmdsConfig {
    firecracker_client::MmdsConfig {
        version: Some("V1".to_string()),
        network_interfaces: network_interface_ids,
        ipv4_address: None,
        imds_compat: Some(true),
    }
}
