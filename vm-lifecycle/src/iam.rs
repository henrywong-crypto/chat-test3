use anyhow::{Context, Result};
use aws_config::default_provider::credentials::DefaultCredentialsChain;
use aws_credential_types::{provider::ProvideCredentials, Credentials};
use firecracker_manager::ImdsCredential;
use std::time::SystemTime;
use time::{format_description::well_known::Rfc3339, OffsetDateTime};
use tracing::{info, warn};

pub struct HostIamCredential {
    pub role_name: String,
    pub credential: ImdsCredential,
}

pub async fn fetch_host_iam_credentials() -> Result<HostIamCredential> {
    let credentials_chain = DefaultCredentialsChain::builder().build().await;
    let credentials = credentials_chain
        .provide_credentials()
        .await
        .context("failed to fetch host IAM credentials")?;
    let role_name = std::env::var("AWS_ROLE_NAME").unwrap_or_else(|_| "vm-role".to_string());
    let expiration = format_credential_expiry(&credentials);
    info!("fetched host IAM credentials");
    Ok(HostIamCredential {
        role_name,
        credential: build_imds_credential(&credentials, expiration),
    })
}

fn system_time_to_iso8601(t: SystemTime) -> Result<String> {
    let dt = OffsetDateTime::try_from(t).context("system time out of range")?;
    dt.format(&Rfc3339)
        .context("failed to format time as RFC 3339")
}

fn format_credential_expiry(credentials: &Credentials) -> String {
    credentials
        .expiry()
        .map(|t| {
            system_time_to_iso8601(t).unwrap_or_else(|e| {
                warn!("failed to format credential expiry: {e}");
                "2099-01-01T00:00:00Z".to_string()
            })
        })
        .unwrap_or_else(|| "2099-01-01T00:00:00Z".to_string())
}

fn build_imds_credential(credentials: &Credentials, expiration: String) -> ImdsCredential {
    let session_token = credentials.session_token().unwrap_or("");
    ImdsCredential::new(
        credentials.access_key_id(),
        credentials.secret_access_key(),
        session_token,
        expiration,
    )
}
