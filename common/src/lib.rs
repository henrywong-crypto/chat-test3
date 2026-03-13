use anyhow::Result;
use std::path::Path;

pub fn validate_within_dir(real_path: &str, allowed_dir: &str) -> Result<()> {
    if !Path::new(real_path).starts_with(allowed_dir) {
        anyhow::bail!("path is outside the allowed directory");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_path_within_dir_passes() {
        assert!(validate_within_dir("/uploads/user/file.txt", "/uploads").is_ok());
    }

    #[test]
    fn test_path_in_nested_subdir_passes() {
        assert!(validate_within_dir("/uploads/a/b/c/d.txt", "/uploads").is_ok());
    }

    #[test]
    fn test_path_equal_to_dir_passes() {
        assert!(validate_within_dir("/uploads", "/uploads").is_ok());
    }

    #[test]
    fn test_path_outside_dir_fails() {
        assert!(validate_within_dir("/etc/passwd", "/uploads").is_err());
    }

    #[test]
    fn test_path_with_shared_string_prefix_but_different_component_fails() {
        // "/uploads-evil" shares the string prefix "/uploads" but is a different
        // directory component — starts_with is component-based, not string-based.
        assert!(validate_within_dir("/uploads-evil/file.txt", "/uploads").is_err());
    }

    #[test]
    fn test_parent_dir_fails() {
        assert!(validate_within_dir("/", "/uploads").is_err());
    }

    #[test]
    fn test_traversal_without_canonicalization_bypasses_check() {
        // Path::starts_with does not resolve "..": the components of
        // "/uploads/../etc/passwd" start with [/, uploads] so the check passes.
        // This documents why callers must canonicalize paths before calling this function.
        assert!(validate_within_dir("/uploads/../etc/passwd", "/uploads").is_ok());
    }
}
