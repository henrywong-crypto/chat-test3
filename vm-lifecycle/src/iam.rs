use anyhow::{Context, Result};
use aws_config::BehaviorVersion;
use aws_credential_types::{provider::ProvideCredentials, Credentials};
use firecracker_manager::ImdsCredential;
use std::time::SystemTime;
use time::{format_description::well_known::Rfc3339, OffsetDateTime};
use tracing::info;

pub struct HostIamCredential {
    pub role_name: String,
    pub credential: ImdsCredential,
}

pub async fn fetch_host_iam_credentials(role_name: &str) -> Result<HostIamCredential> {
    let config = aws_config::load_defaults(BehaviorVersion::v2023_11_09()).await;
    let credentials = config
        .credentials_provider()
        .context("no credentials provider configured")?
        .provide_credentials()
        .await
        .context("failed to fetch host IAM credentials")?;
    let expiration = format_credential_expiry(&credentials)?;
    info!("fetched host IAM credentials");
    Ok(HostIamCredential {
        role_name: role_name.to_owned(),
        credential: build_imds_credential(&credentials, expiration)?,
    })
}

fn system_time_to_iso8601(t: SystemTime) -> Result<String> {
    let dt = OffsetDateTime::try_from(t).context("system time out of range")?;
    dt.format(&Rfc3339)
        .context("failed to format time as RFC 3339")
}

fn format_credential_expiry(credentials: &Credentials) -> Result<String> {
    credentials
        .expiry()
        .map(system_time_to_iso8601)
        .transpose()?
        .context("missing credential expiry")
}

fn build_imds_credential(credentials: &Credentials, expiration: String) -> Result<ImdsCredential> {
    let session_token = credentials.session_token().context("missing session token")?;
    Ok(ImdsCredential::new(
        credentials.access_key_id(),
        credentials.secret_access_key(),
        session_token,
        expiration,
    ))
}
