//! Effective mounts of a sandbox and the rules a mount path must satisfy (contract C10).

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// One persistent root mounted (or about to be) into a sandbox.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct MountSpec {
    pub root_id: Uuid,
    pub mount_path: String,
    pub read_only: bool,
}

/// What a sandbox has mounted and what it could mount.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
pub struct EffectiveMounts {
    pub roots: Vec<serde_json::Value>,
    pub filesystem: serde_json::Value,
    pub available_roots: Vec<serde_json::Value>,
    pub needs_recreate: bool,
}

/// Check a mount path: absolute, no `..`, no empty segments, no NUL.
pub fn validate_mount_path(path: &str) -> Result<(), String> {
    if !path.starts_with('/') {
        return Err("The mount path must be absolute.".into());
    }
    if path == "/" {
        return Err("The mount path cannot be the root directory.".into());
    }
    if path.contains('\0') {
        return Err("The mount path contains a NUL byte.".into());
    }
    if path.len() > 512 {
        return Err("The mount path is longer than 512 characters.".into());
    }
    for segment in path.trim_end_matches('/').split('/').skip(1) {
        if segment.is_empty() {
            return Err("The mount path has an empty segment.".into());
        }
        if segment == ".." || segment == "." {
            return Err("The mount path must not contain '.' or '..'.".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn good_paths_pass() {
        assert!(validate_mount_path("/work/customer-portal").is_ok());
        assert!(validate_mount_path("/data/").is_ok());
    }

    #[test]
    fn bad_paths_are_refused() {
        for path in ["work", "/", "/a/../b", "/a//b", "/a/./b", ""] {
            assert!(validate_mount_path(path).is_err(), "{path} should be refused");
        }
    }
}
