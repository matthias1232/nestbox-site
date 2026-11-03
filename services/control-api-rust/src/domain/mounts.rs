//! Effective mounts of a sandbox and the rules a mount path must satisfy (contract C10).

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// One persistent root mounted (or about to be) into a sandbox.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct MountSpec {
    /// The persistent root to bind.
    pub root_id: Uuid,
    /// Where it appears inside the sandbox.
    pub mount_path: String,
    /// Whether the sandbox may only read it.
    pub read_only: bool,
}

/// What a sandbox has mounted and what it could mount.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
pub struct EffectiveMounts {
    /// Mounted roots with their owner, node and state.
    pub roots: Vec<serde_json::Value>,
    /// The filesystem policy's read-only and read-write paths and where they come from.
    pub filesystem: serde_json::Value,
    /// Roots on the sandbox's node that are free to mount here.
    pub available_roots: Vec<serde_json::Value>,
    /// True when a mount row is not yet in the running container.
    pub needs_recreate: bool,
}

/// The difference between the current mount set and a desired one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MountDiff {
    /// Roots in the desired set that are not mounted now.
    pub add: Vec<MountSpec>,
    /// Roots mounted now that the desired set leaves out.
    pub remove: Vec<MountSpec>,
    /// Roots in both whose path or access differs; carries the desired values.
    pub change: Vec<MountSpec>,
}

impl MountDiff {
    /// True when nothing has to be mounted, dismounted or changed.
    pub fn is_empty(&self) -> bool {
        self.add.is_empty() && self.remove.is_empty() && self.change.is_empty()
    }
}

impl PartialEq for MountSpec {
    fn eq(&self, other: &Self) -> bool {
        self.root_id == other.root_id
            && self.mount_path == other.mount_path
            && self.read_only == other.read_only
    }
}

impl Eq for MountSpec {}

/// Compare the mounts a sandbox has with the ones it should have, keyed by root.
pub fn diff(current: &[MountSpec], desired: &[MountSpec]) -> MountDiff {
    let mut out = MountDiff::default();
    for want in desired {
        match current.iter().find(|have| have.root_id == want.root_id) {
            None => out.add.push(want.clone()),
            Some(have) if have != want => out.change.push(want.clone()),
            Some(_) => {}
        }
    }
    for have in current {
        if !desired.iter().any(|want| want.root_id == have.root_id) {
            out.remove.push(have.clone());
        }
    }
    out
}

/// Paths a mount may never cover: they hold secrets or kernel interfaces.
const FORBIDDEN_PREFIXES: [&str; 4] = ["/run/secrets", "/proc", "/sys", "/dev"];



/// Check a mount path: absolute, no `..`, no empty segments, no trailing slash, no NUL,
/// at most 255 characters, not `/`, and not under a protected system directory.
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
    if path.len() > 255 {
        return Err("The mount path is longer than 255 characters.".into());
    }
    if path.ends_with('/') {
        return Err("The mount path must not end with a slash.".into());
    }
    for segment in path.split('/').skip(1) {
        if segment.is_empty() {
            return Err("The mount path has an empty segment.".into());
        }
        if segment == ".." || segment == "." {
            return Err("The mount path must not contain '.' or '..'.".into());
        }
    }
    for prefix in FORBIDDEN_PREFIXES {
        if path == prefix || path.starts_with(&format!("{prefix}/")) {
            return Err(format!("The mount path must not be under {prefix}."));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(root: u128, path: &str, read_only: bool) -> MountSpec {
        MountSpec { root_id: Uuid::from_u128(root), mount_path: path.into(), read_only }
    }

    #[test]
    fn good_paths_pass() {
        assert!(validate_mount_path("/work/customer-portal").is_ok());
        assert!(validate_mount_path("/data").is_ok());
    }

    #[test]
    fn bad_paths_are_refused() {
        for path in [
            "work", "/", "/a/../b", "/a//b", "/a/./b", "", "/data/", "/proc", "/proc/x",
            "/sys/kernel", "/dev/shm", "/run/secrets/a",
        ] {
            assert!(validate_mount_path(path).is_err(), "{path} should be refused");
        }
        assert!(validate_mount_path(&format!("/{}", "a".repeat(255))).is_err());
        assert!(validate_mount_path("/procedure").is_ok());
    }

    #[test]
    fn diff_sorts_roots_into_add_remove_change() {
        let current = vec![spec(1, "/a", false), spec(2, "/b", false), spec(3, "/c", true)];
        let desired = vec![spec(2, "/b", true), spec(3, "/c", true), spec(4, "/d", false)];
        let d = diff(&current, &desired);
        assert_eq!(d.add, vec![spec(4, "/d", false)]);
        assert_eq!(d.remove, vec![spec(1, "/a", false)]);
        assert_eq!(d.change, vec![spec(2, "/b", true)]);
    }

    #[test]
    fn equal_sets_have_an_empty_diff() {
        let set = vec![spec(1, "/a", false)];
        assert!(diff(&set, &set).is_empty());
    }
}
