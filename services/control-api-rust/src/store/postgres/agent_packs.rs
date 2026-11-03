//! Baselines, agent packs, and the tool catalog.
//!
//! All three tables are global catalog data; no environment filter applies.

use sqlx::Row;

use super::super::TenantScope;
use crate::domain::pack::{AgentPack, Baseline, InstallKind, PackageManager, Tool};
use crate::error::StoreError;

fn row_to_baseline(row: &sqlx::postgres::PgRow) -> Result<Baseline, StoreError> {
    let pm_raw: String = row.try_get("package_manager")?;
    Ok(Baseline {
        key: row.try_get("key")?,
        name: row.try_get("name")?,
        recommended: row.try_get("recommended")?,
        base_image: row.try_get("base_image")?,
        package_manager: PackageManager::parse(&pm_raw).unwrap_or(PackageManager::Apt),
        default_tools: row.try_get("default_tools")?,
        openshell_ready: row.try_get("openshell_ready")?,
        notes: row.try_get("notes")?,
    })
}

const BASELINE_COLUMNS: &str =
    "key, name, recommended, base_image, package_manager, default_tools, openshell_ready, notes";

/// Every baseline, recommended first.
pub async fn list_baselines(scope: &mut TenantScope<'_>) -> Result<Vec<Baseline>, StoreError> {
    let sql = format!("SELECT {BASELINE_COLUMNS} FROM baselines ORDER BY recommended DESC, name");
    let rows = sqlx::query(&sql).fetch_all(scope.connection()).await?;
    rows.iter().map(row_to_baseline).collect()
}

/// One baseline by key.
pub async fn get_baseline(scope: &mut TenantScope<'_>, key: &str) -> Result<Baseline, StoreError> {
    let sql = format!("SELECT {BASELINE_COLUMNS} FROM baselines WHERE key = $1");
    let row = sqlx::query(&sql)
        .bind(key)
        .fetch_optional(scope.connection())
        .await?
        .ok_or(StoreError::NotFound)?;
    row_to_baseline(&row)
}

const PACK_COLUMNS: &str = "key, name, vendor, description, base_os, install_kind, install_ref, \
     home_paths, requires_gpu, needs_inference, default_image, default_command, ports, \
     env_slots, enabled";

fn row_to_pack(row: &sqlx::postgres::PgRow) -> Result<AgentPack, StoreError> {
    let kind_raw: String = row.try_get("install_kind")?;
    Ok(AgentPack {
        key: row.try_get("key")?,
        name: row.try_get("name")?,
        vendor: row.try_get("vendor")?,
        description: row.try_get("description")?,
        base_os: row.try_get("base_os")?,
        install_kind: InstallKind::parse(&kind_raw).unwrap_or(InstallKind::None),
        install_ref: row.try_get("install_ref")?,
        home_paths: row.try_get("home_paths")?,
        requires_gpu: row.try_get("requires_gpu")?,
        needs_inference: row.try_get("needs_inference")?,
        default_image: row.try_get("default_image")?,
        default_command: row.try_get("default_command")?,
        ports: row.try_get("ports")?,
        env_slots: row.try_get("env_slots")?,
        enabled: row.try_get("enabled")?,
    })
}

/// Every enabled pack.
pub async fn list_packs(scope: &mut TenantScope<'_>) -> Result<Vec<AgentPack>, StoreError> {
    let sql = format!("SELECT {PACK_COLUMNS} FROM agent_packs WHERE enabled ORDER BY name");
    let rows = sqlx::query(&sql).fetch_all(scope.connection()).await?;
    rows.iter().map(row_to_pack).collect()
}

/// One pack by key, enabled or not.
pub async fn get_pack(scope: &mut TenantScope<'_>, key: &str) -> Result<AgentPack, StoreError> {
    let sql = format!("SELECT {PACK_COLUMNS} FROM agent_packs WHERE key = $1");
    let row = sqlx::query(&sql)
        .bind(key)
        .fetch_optional(scope.connection())
        .await?
        .ok_or(StoreError::NotFound)?;
    row_to_pack(&row)
}

const TOOL_COLUMNS: &str = "key, name, \"group\", description, apt_name, rpm_name, apk_name";

fn row_to_tool(row: &sqlx::postgres::PgRow) -> Result<Tool, StoreError> {
    Ok(Tool {
        key: row.try_get("key")?,
        name: row.try_get("name")?,
        group: row.try_get("group")?,
        description: row.try_get("description")?,
        apt_name: row.try_get("apt_name")?,
        rpm_name: row.try_get("rpm_name")?,
        apk_name: row.try_get("apk_name")?,
    })
}

/// The whole tool catalog, grouped order.
pub async fn list_tools(scope: &mut TenantScope<'_>) -> Result<Vec<Tool>, StoreError> {
    let sql = format!("SELECT {TOOL_COLUMNS} FROM tool_catalog ORDER BY \"group\", name");
    let rows = sqlx::query(&sql).fetch_all(scope.connection()).await?;
    rows.iter().map(row_to_tool).collect()
}

/// Tools by key, in the order asked for; unknown keys are skipped.
pub async fn tools_by_key(
    scope: &mut TenantScope<'_>,
    keys: &[String],
) -> Result<Vec<Tool>, StoreError> {
    let sql = format!("SELECT {TOOL_COLUMNS} FROM tool_catalog WHERE key = ANY($1)");
    let rows = sqlx::query(&sql)
        .bind(keys)
        .fetch_all(scope.connection())
        .await?;
    rows.iter().map(row_to_tool).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    /// A pool from `TEST_DATABASE_URL`, with the migrations applied, or `None`
    /// when no disposable database is configured.
    async fn test_pool() -> Option<sqlx::PgPool> {
        let url = std::env::var("TEST_DATABASE_URL").ok()?;
        let pool = sqlx::PgPool::connect(&url).await.ok()?;
        sqlx::migrate!("./migrations").run(&pool).await.ok()?;
        Some(pool)
    }

    /// Every seeded pack must carry an image and a command.
    ///
    /// The sandbox service now takes both from these rows instead of a `match`
    /// (ADR 0020), so a missing value stopped being invisible: an empty image
    /// cannot be pulled, and an empty command silently runs whatever the
    /// image's entrypoint does. A pack that is its own image
    /// (`install_kind = image`) is the documented exception — its entrypoint
    /// *is* the command, which is what the old `match` returned for openclaw.
    ///
    /// Runs against the real seed when `TEST_DATABASE_URL` is set and skips
    /// otherwise, like the rest of the database checks.
    #[tokio::test]
    async fn every_seeded_pack_has_an_image_and_a_command() {
        let Some(pool) = test_pool().await else {
            eprintln!("skipping: TEST_DATABASE_URL is not set");
            return;
        };
        let mut scope = TenantScope::begin(&pool, Uuid::nil())
            .await
            .expect("tenant scope");
        let packs = list_packs(&mut scope).await.expect("packs");
        scope.commit().await.expect("commit");

        assert_eq!(packs.len(), 10, "the migrations seed ten packs (nine agents and the desktop)");
        for pack in &packs {
            assert!(
                !pack.default_image.trim().is_empty(),
                "pack '{}' has no default image",
                pack.key
            );
            if pack.install_kind == InstallKind::Image {
                continue;
            }
            assert!(
                !pack.default_command.is_empty(),
                "pack '{}' has no default command",
                pack.key
            );
        }
    }

    /// The compatibility field must be usable as written: every `base_os` entry
    /// names a baseline, or the pack would be offered for an OS that does not
    /// exist and filtered out for every real one.
    #[tokio::test]
    async fn every_pack_base_os_names_a_known_baseline() {
        let Some(pool) = test_pool().await else {
            eprintln!("skipping: TEST_DATABASE_URL is not set");
            return;
        };
        let unknown: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM agent_packs p, unnest(p.base_os) AS os
              WHERE NOT EXISTS (SELECT 1 FROM baselines b WHERE b.key = os)",
        )
        .fetch_one(&pool)
        .await
        .expect("query");
        assert_eq!(unknown, 0, "a pack names a baseline that does not exist");
    }
}
