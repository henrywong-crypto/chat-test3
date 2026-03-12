use anyhow::Result;

pub mod file;
pub mod seekable_channel_writer;
pub mod zip;

use std::path::Path;

pub fn validate_within_dir(real_path: &str, allowed_dir: &str) -> Result<()> {
    if !Path::new(real_path).starts_with(allowed_dir) {
        anyhow::bail!("path is outside the allowed directory");
    }
    Ok(())
}
