//! Helpers for passing IAM role credentials to Firecracker guests via MMDS in
//! EC2 IMDS-compatible format so the AWS SDK (e.g. Rust default credential chain) works.
//!
//! See [docs/mmds-iam-role.md](../../docs/mmds-iam-role.md) for the full design.

use std::time::SystemTime;

use chrono::{DateTime, Utc};
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
            last_updated: format_iso8601_now(),
            type_: "AWS-HMAC".to_string(),
            access_key_id: access_key_id.into(),
            secret_access_key: secret_access_key.into(),
            token: token.into(),
            expiration: expiration.into(),
        }
    }
}

fn format_iso8601_now() -> String {
    Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

pub fn system_time_to_iso8601(t: SystemTime) -> String {
    let dt: DateTime<Utc> = t.into();
    dt.format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

/// Build full MMDS JSON for initial `PUT /mmds`: instance-id + IAM credentials
/// in EC2 IMDS shape. Use with `imds_compat: true` and `version: "V2"` in MmdsConfig.
pub fn build_mmds_with_iam(
    instance_id: &str,
    role_name: &str,
    credential: &ImdsCredential,
) -> serde_json::Value {
    // Store credentials as a JSON string (leaf node), not a nested object.
    // MMDS treats nested objects as directories and returns key listings instead
    // of JSON, which breaks the AWS CLI and SDK credential parsers.
    let cred_str = serde_json::to_string(credential).unwrap();
    let mut security_credentials = serde_json::Map::new();
    security_credentials.insert(role_name.to_string(), serde_json::Value::String(cred_str));
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
pub fn build_mmds_iam_refresh_patch(
    role_name: &str,
    credential: &ImdsCredential,
) -> serde_json::Value {
    let cred_str = serde_json::to_string(credential).unwrap();
    let mut security_credentials = serde_json::Map::new();
    security_credentials.insert(role_name.to_string(), serde_json::Value::String(cred_str));
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
pub fn imds_compat_mmds_config(
    network_interface_ids: Vec<String>,
) -> firecracker_client::MmdsConfig {
    firecracker_client::MmdsConfig {
        version: Some("V1".to_string()),
        network_interfaces: network_interface_ids,
        ipv4_address: None,
        imds_compat: Some(true),
    }
}
