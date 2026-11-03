//! Effective mounts of a sandbox: one readable shape and one writable set (package B09, C10).
//!
//! Reading joins three facts the console used to fetch separately: the persistent roots mounted
//! here, the roots that could be, and the filesystem paths the sandbox's policy grants. Writing
//! diffs the desired set against the current one and calls the existing mount and dismount
//! services per difference, so every rule they enforce (one read-write mount per root, a root
//! only on its own node) still holds. A bind mount is fixed when the container is created, so a
//! change takes effect with one recreate, either now or whenever the operator chooses.

use std::collections::{BTreeSet, HashMap};

use uuid::Uuid;

use super::persist_service::{self, MountRequest};
use super::{require, ServiceContext};
use crate::domain::auth::{Permission, Principal};
use crate::domain::mounts::{self, EffectiveMounts, MountSpec};
use crate::domain::persist::PersistentRoot;
use crate::domain::sandbox::Sandbox;
use crate::error::{ApiError, ErrorCode};
use crate::store::TenantScope;
use crate::store::postgres::persistent_roots as root_store;
use crate::store::postgres::sandboxes as sandbox_store;

/// The read-only paths the runner gives every sandbox when no policy states its own
/// (`FILESYSTEM_POLICY` in `sandbox-runner/app/openshell_backend.py`).
const RUNNER_READ_ONLY: [&str; 12] = [
    "/bin", "/usr", "/lib", "/proc", "/dev/urandom", "/dev/random", "/etc", "/var/log", "/opt",
    "/var/lib", "/sys", "/run/secrets",
];

/// The read-write paths the runner gives every sandbox when no policy states its own.
const RUNNER_READ_WRITE: [&str; 9] = [
    "/tmp", "/var/tmp", "/dev/null", "/dev/zero", "/dev/tty", "/dev/ptmx", "/dev/pts", "/dev/shm",
    "/var/lib/xkb",
];

/// What the sandbox has mounted, what it could mount, and whether a recreate is pending.
pub async fn effective(
    ctx: &ServiceContext,
    principal: &Principal,
    sandbox_id: Uuid,
) -> Result<EffectiveMounts, ApiError> {
    require(principal, Permission::SandboxRead)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let sandbox = sandbox_store::get(&mut scope, sandbox_id)
        .await
        .map_err(ApiError::from)?;
    let view = build(&mut scope, &sandbox).await?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(view)
}

/// Make the mount set equal `roots`; with `apply == "now"` recreate the sandbox.
pub async fn set(
    ctx: &ServiceContext,
    principal: &Principal,
    sandbox_id: Uuid,
    roots: Vec<MountSpec>,
    apply: &str,
) -> Result<EffectiveMounts, ApiError> {
    require(principal, Permission::PersistWrite)?;
    validate_set(&roots)?;

    // Read the current set and check every wanted root before anything changes.
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let sandbox = sandbox_store::get(&mut scope, sandbox_id)
        .await
        .map_err(ApiError::from)?;
    let current: Vec<MountSpec> = root_store::mounts_for_sandbox(&mut scope, sandbox.id)
        .await
        .map_err(ApiError::from)?
        .into_iter()
        .map(|(mount, _)| MountSpec {
            root_id: mount.root_id,
            mount_path: mount.mount_path,
            read_only: mount.read_only,
        })
        .collect();
    for wanted in &roots {
        let root = root_store::get_root(&mut scope, wanted.root_id)
            .await
            .map_err(|error| match error {
                crate::error::StoreError::NotFound => ApiError::not_found(
                    ErrorCode::ROOT_NOT_FOUND,
                    "No persistent root with that id.",
                ),
                other => ApiError::from(other),
            })?;
        if root.archived_at.is_some() {
            return Err(ApiError::not_found(
                ErrorCode::ROOT_NOT_FOUND,
                "That root is archived and cannot take a new mount.",
            ));
        }
        if root.node_id != sandbox.node_id {
            return Err(ApiError::unprocessable(
                ErrorCode("mount.wrong_node"),
                format!("Root '{}' lives on another node than this sandbox.", root.slug),
            ));
        }
    }
    scope.commit().await.map_err(ApiError::from)?;

    let difference = mounts::diff(&current, &roots);

    // Remove first so a read-write root freed here can be taken by a change or an add.
    for gone in &difference.remove {
        persist_service::dismount(ctx, principal, gone.root_id, sandbox.id).await?;
    }
    for changed in &difference.change {
        persist_service::dismount(ctx, principal, changed.root_id, sandbox.id).await?;
        persist_service::mount(ctx, principal, changed.root_id, request_for(sandbox.id, changed))
            .await?;
    }
    for added in &difference.add {
        persist_service::mount(ctx, principal, added.root_id, request_for(sandbox.id, added))
            .await?;
    }

    if !difference.is_empty() {
        audit_change(ctx, principal, &sandbox, &difference).await?;
    }
    if apply == "now" && !difference.is_empty() {
        super::sandbox_service::recreate(ctx, principal, sandbox.id).await?;
    }
    effective(ctx, principal, sandbox.id).await
}

/// The mount request the persistence service takes for one wanted mount.
fn request_for(sandbox_id: Uuid, spec: &MountSpec) -> MountRequest {
    MountRequest {
        sandbox_id,
        mount_path: spec.mount_path.clone(),
        read_only: spec.read_only,
    }
}

/// Refuse a bad path, a path used twice, or a root listed twice, before any write.
fn validate_set(roots: &[MountSpec]) -> Result<(), ApiError> {
    let mut paths = BTreeSet::new();
    let mut ids = BTreeSet::new();
    for spec in roots {
        mounts::validate_mount_path(&spec.mount_path).map_err(ApiError::validation)?;
        if !paths.insert(spec.mount_path.clone()) {
            return Err(ApiError::validation(format!(
                "The mount path {} is used twice.",
                spec.mount_path
            )));
        }
        if !ids.insert(spec.root_id) {
            return Err(ApiError::validation("A root can be mounted only once per sandbox."));
        }
    }
    Ok(())
}

/// Record what the change did to the mount set.
async fn audit_change(
    ctx: &ServiceContext,
    principal: &Principal,
    sandbox: &Sandbox,
    difference: &mounts::MountDiff,
) -> Result<(), ApiError> {
    let render = |list: &[MountSpec]| {
        list.iter()
            .map(|spec| {
                serde_json::json!({
                    "root_id": spec.root_id,
                    "mount_path": spec.mount_path,
                    "read_only": spec.read_only,
                })
            })
            .collect::<Vec<_>>()
    };
    let detail = serde_json::json!({
        "add": render(&difference.add),
        "remove": render(&difference.remove),
        "change": render(&difference.change),
    });
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    crate::store::postgres::audit::record(
        &mut scope,
        crate::store::postgres::audit::NewAuditEvent {
            actor_id: Some(principal.user_id),
            actor_label: &principal.display_name,
            kind: "sandbox.mounts_changed",
            subject_type: Some("sandbox"),
            subject_id: Some(sandbox.id),
            detail: &detail,
            request_id: None,
        },
    )
    .await
    .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)
}

/// Assemble the C10 answer for one sandbox inside an open scope.
async fn build(
    scope: &mut TenantScope<'_>,
    sandbox: &Sandbox,
) -> Result<EffectiveMounts, ApiError> {
    let mounted = root_store::mounts_for_sandbox(scope, sandbox.id)
        .await
        .map_err(ApiError::from)?;
    let all_roots = root_store::list_roots(scope).await.map_err(ApiError::from)?;
    let by_id: HashMap<Uuid, &PersistentRoot> = all_roots.iter().map(|r| (r.id, r)).collect();

    // The container's own view, when an inventory exists: the targets it was created with.
    let running = running_targets(scope, sandbox.id).await?;

    let mut owner_ids: Vec<Uuid> = all_roots.iter().filter_map(|r| r.owner_user_id).collect();
    owner_ids.sort();
    owner_ids.dedup();
    let owners = owner_names(scope, &owner_ids).await?;
    let node = sandbox.node_name.clone().unwrap_or_default();

    let mut needs_recreate = false;
    let mut roots = Vec::new();
    for (mount, _host_path) in &mounted {
        let root = by_id.get(&mount.root_id);
        let pending = running
            .as_ref()
            .is_some_and(|targets| !targets.contains(&mount.mount_path));
        needs_recreate |= pending;
        roots.push(serde_json::json!({
            "root_id": mount.root_id,
            "slug": root.map(|r| r.slug.as_str()).unwrap_or(""),
            "kind": root.map(|r| r.kind.as_str()).unwrap_or("project"),
            "owner": owner_label(root.and_then(|r| r.owner_user_id), &owners),
            "node": node,
            "mount_path": mount.mount_path,
            "read_only": mount.read_only,
            "state": if pending { "pending_recreate" } else { "mounted" },
        }));
    }

    let mounted_ids: BTreeSet<Uuid> = mounted.iter().map(|(m, _)| m.root_id).collect();
    let available_roots: Vec<serde_json::Value> = all_roots
        .iter()
        .filter(|r| r.node_id == sandbox.node_id && !mounted_ids.contains(&r.id))
        .map(|r| {
            serde_json::json!({
                "root_id": r.id,
                "slug": r.slug,
                "kind": r.kind.as_str(),
                "owner": owner_label(r.owner_user_id, &owners),
                "node": node,
                "bytes_used": r.bytes_used,
            })
        })
        .collect();

    let filesystem = filesystem_paths(scope, sandbox.id).await?;
    Ok(EffectiveMounts { roots, filesystem, available_roots, needs_recreate })
}

/// The mount targets the running container has, from the stored inventory; `None` when no
/// inventory was taken, because then nothing can be said about the container.
async fn running_targets(
    scope: &mut TenantScope<'_>,
    sandbox_id: Uuid,
) -> Result<Option<BTreeSet<String>>, ApiError> {
    let inventory = crate::store::postgres::metrics::read_inventory(scope, sandbox_id)
        .await
        .map_err(ApiError::from)?;
    Ok(inventory.map(|inv| {
        inv.mounts
            .as_array()
            .map(|list| {
                list.iter()
                    .filter_map(|m| m.get("target").and_then(|t| t.as_str()))
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    }))
}

/// Display names of the given users; an unknown id simply has no entry.
async fn owner_names(
    scope: &mut TenantScope<'_>,
    ids: &[Uuid],
) -> Result<HashMap<Uuid, String>, ApiError> {
    use sqlx::Row;
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows = sqlx::query("SELECT id, display_name FROM users WHERE id = ANY($1)")
        .bind(ids)
        .fetch_all(scope.connection())
        .await
        .map_err(|error| ApiError::from(crate::error::StoreError::from(error)))?;
    let mut out = HashMap::new();
    for row in rows {
        let id: Uuid = row
            .try_get("id")
            .map_err(|error| ApiError::from(crate::error::StoreError::from(error)))?;
        let name: String = row
            .try_get("display_name")
            .map_err(|error| ApiError::from(crate::error::StoreError::from(error)))?;
        out.insert(id, name);
    }
    Ok(out)
}

/// The owner's name, or JSON null for a root nobody owns.
fn owner_label(owner: Option<Uuid>, names: &HashMap<Uuid, String>) -> serde_json::Value {
    match owner {
        Some(id) => names
            .get(&id)
            .map(|name| serde_json::json!(name))
            .unwrap_or_else(|| serde_json::json!(id.to_string())),
        None => serde_json::Value::Null,
    }
}

/// The read-only and read-write paths of the sandbox's filesystem policy, or the runner's
/// defaults when the assigned policy states none.
async fn filesystem_paths(
    scope: &mut TenantScope<'_>,
    sandbox_id: Uuid,
) -> Result<serde_json::Value, ApiError> {
    let assignment = crate::store::postgres::policies::assignment_for_sandbox(scope, sandbox_id)
        .await
        .map_err(ApiError::from)?;
    if let Some(assignment) = assignment {
        let policy = crate::store::postgres::policies::get(scope, assignment.policy_id)
            .await
            .map_err(ApiError::from)?;
        if let Some((read_only, read_write)) = paths_from_spec(&policy.spec) {
            return Ok(serde_json::json!({
                "read_only": read_only,
                "read_write": read_write,
                "source": "policy",
            }));
        }
    }
    Ok(serde_json::json!({
        "read_only": RUNNER_READ_ONLY,
        "read_write": RUNNER_READ_WRITE,
        "source": "runner_default",
    }))
}

/// Read the paths out of a policy document's `filesystem_policy`, in either form: the runner's
/// two lists, or a list of `{ path, access }` grants. `None` when the section grants nothing.
fn paths_from_spec(spec: &serde_json::Value) -> Option<(Vec<String>, Vec<String>)> {
    let section = spec.get("filesystem_policy")?;
    let strings = |value: Option<&serde_json::Value>| -> Vec<String> {
        value
            .and_then(|v| v.as_array())
            .map(|list| list.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
            .unwrap_or_default()
    };
    let mut read_only = strings(section.get("read_only"));
    let mut read_write = strings(section.get("read_write"));
    if let Some(grants) = section.get("paths").and_then(|v| v.as_array()) {
        for grant in grants {
            let Some(path) = grant.get("path").and_then(|v| v.as_str()) else { continue };
            match grant.get("access").and_then(|v| v.as_str()) {
                Some("read") => read_only.push(path.to_string()),
                Some("write") | Some("read_write") => read_write.push(path.to_string()),
                _ => {}
            }
        }
    }
    if read_only.is_empty() && read_write.is_empty() {
        None
    } else {
        Some((read_only, read_write))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_runner_form_of_a_policy_is_read_as_two_lists() {
        let spec = serde_json::json!({
            "filesystem_policy": { "read_only": ["/etc/ssl"], "read_write": ["/home/nestbox"] }
        });
        let (ro, rw) = paths_from_spec(&spec).expect("paths");
        assert_eq!(ro, vec!["/etc/ssl"]);
        assert_eq!(rw, vec!["/home/nestbox"]);
    }

    #[test]
    fn path_grants_are_sorted_by_access() {
        let spec = serde_json::json!({
            "filesystem_policy": { "paths": [
                { "path": "/a", "access": "read" },
                { "path": "/b", "access": "read_write" },
                { "path": "/c", "access": "none" },
            ]}
        });
        let (ro, rw) = paths_from_spec(&spec).expect("paths");
        assert_eq!(ro, vec!["/a"]);
        assert_eq!(rw, vec!["/b"]);
    }

    #[test]
    fn an_empty_section_falls_back_to_the_runner_default() {
        assert!(paths_from_spec(&serde_json::json!({ "filesystem_policy": {} })).is_none());
        assert!(paths_from_spec(&serde_json::json!({})).is_none());
    }

    #[test]
    fn a_set_with_a_repeated_path_or_root_is_refused() {
        let a = MountSpec { root_id: Uuid::from_u128(1), mount_path: "/a".into(), read_only: false };
        let b = MountSpec { root_id: Uuid::from_u128(2), mount_path: "/a".into(), read_only: false };
        assert!(validate_set(&[a.clone(), b]).is_err());
        assert!(validate_set(&[a.clone(), a]).is_err());
    }
}
