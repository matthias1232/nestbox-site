//! The exit proxy catalogue.
//!
//! Reads and writes happen inside the tenant transaction like every other
//! aggregate; the row-level policy on `proxies` repeats the environment check so
//! a forgotten filter returns nothing rather than another tenant's exits.

use sqlx::Row;
use uuid::Uuid;

use super::super::TenantScope;
use crate::domain::proxy::{NewProxy, Proxy, ProxyKind};
use crate::error::StoreError;

const COLUMNS: &str = "id, environment_id, name, kind, host, port, country, provider, \
     username, password_ref, enabled, version, created_at, updated_at";

fn row_to_proxy(row: &sqlx::postgres::PgRow) -> Result<Proxy, StoreError> {
    let kind_raw: String = row.try_get("kind")?;
    let password_ref: Option<String> = row.try_get("password_ref")?;
    Ok(Proxy {
        id: row.try_get("id")?,
        environment_id: row.try_get("environment_id")?,
        name: row.try_get("name")?,
        kind: ProxyKind::parse(&kind_raw).unwrap_or(ProxyKind::Http),
        host: row.try_get("host")?,
        port: row.try_get("port")?,
        country: row.try_get("country")?,
        provider: row.try_get("provider")?,
        username: row.try_get("username")?,
        password_configured: password_ref.is_some(),
        password_ref,
        enabled: row.try_get("enabled")?,
        version: row.try_get("version")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

/// Every exit in this environment, disabled ones included.
///
/// A disabled exit is listed on purpose: the console has to show what an
/// operator switched off, and a sandbox still pointing at it must be visible
/// rather than silently falling back.
pub async fn list(scope: &mut TenantScope<'_>) -> Result<Vec<Proxy>, StoreError> {
    let sql = format!(
        "SELECT {COLUMNS} FROM proxies WHERE environment_id = $1 ORDER BY country NULLS LAST, name"
    );
    let rows = sqlx::query(&sql)
        .bind(scope.environment_id())
        .fetch_all(scope.connection())
        .await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(row_to_proxy(&row)?);
    }
    Ok(out)
}

/// One exit by id.
pub async fn get(scope: &mut TenantScope<'_>, id: Uuid) -> Result<Proxy, StoreError> {
    let sql = format!("SELECT {COLUMNS} FROM proxies WHERE environment_id = $1 AND id = $2");
    let row = sqlx::query(&sql)
        .bind(scope.environment_id())
        .bind(id)
        .fetch_optional(scope.connection())
        .await?
        .ok_or(StoreError::NotFound)?;
    row_to_proxy(&row)
}

/// Insert an exit.
pub async fn insert(scope: &mut TenantScope<'_>, new: &NewProxy) -> Result<Proxy, StoreError> {
    let sql = format!(
        "INSERT INTO proxies
            (environment_id, name, kind, host, port, country, provider, username, password_ref)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
         RETURNING {COLUMNS}"
    );
    let row = sqlx::query(&sql)
        .bind(scope.environment_id())
        .bind(&new.name)
        .bind(new.kind.as_str())
        .bind(&new.host)
        .bind(new.port)
        .bind(&new.country)
        .bind(&new.provider)
        .bind(&new.username)
        .bind(&new.password_ref)
        .fetch_one(scope.connection())
        .await?;
    row_to_proxy(&row)
}

/// Replace an exit's editable fields, guarded by its version.
pub async fn update(
    scope: &mut TenantScope<'_>,
    id: Uuid,
    expected_version: i64,
    new: &NewProxy,
    enabled: bool,
) -> Result<Proxy, StoreError> {
    let sql = format!(
        "WITH updated AS (
            UPDATE proxies
               SET name = $3, kind = $4, host = $5, port = $6, country = $7,
                   provider = $8, username = $9, password_ref = $10,
                   enabled = $11, version = version + 1, updated_at = now()
             WHERE environment_id = $1 AND id = $2 AND version = $12
         RETURNING {COLUMNS}
         )
         SELECT {COLUMNS} FROM updated"
    );
    let row = sqlx::query(&sql)
        .bind(scope.environment_id())
        .bind(id)
        .bind(&new.name)
        .bind(new.kind.as_str())
        .bind(&new.host)
        .bind(new.port)
        .bind(&new.country)
        .bind(&new.provider)
        .bind(&new.username)
        .bind(&new.password_ref)
        .bind(enabled)
        .bind(expected_version)
        .fetch_optional(scope.connection())
        .await?
        .ok_or(StoreError::VersionConflict)?;
    row_to_proxy(&row)
}

/// Remove an exit. A sandbox or provider that pointed at it falls back to the
/// node's own route, which is what the foreign key's `SET NULL` does.
pub async fn delete(scope: &mut TenantScope<'_>, id: Uuid) -> Result<(), StoreError> {
    let result = sqlx::query("DELETE FROM proxies WHERE environment_id = $1 AND id = $2")
        .bind(scope.environment_id())
        .bind(id)
        .execute(scope.connection())
        .await?;
    if result.rows_affected() == 0 {
        return Err(StoreError::NotFound);
    }
    Ok(())
}

/// The exit a sandbox leaves through, when it has one.
pub async fn for_sandbox(
    scope: &mut TenantScope<'_>,
    sandbox_id: Uuid,
) -> Result<Option<Proxy>, StoreError> {
    let sql = format!(
        "SELECT {COLUMNS} FROM proxies
          WHERE environment_id = $1
            AND id = (SELECT proxy_id FROM sandboxes WHERE id = $2 AND environment_id = $1)"
    );
    let row = sqlx::query(&sql)
        .bind(scope.environment_id())
        .bind(sandbox_id)
        .fetch_optional(scope.connection())
        .await?;
    row.map(|row| row_to_proxy(&row)).transpose()
}

/// Point a sandbox at an exit, or at the node's own route when `proxy_id` is
/// `None`.
pub async fn set_sandbox_proxy(
    scope: &mut TenantScope<'_>,
    sandbox_id: Uuid,
    proxy_id: Option<Uuid>,
) -> Result<(), StoreError> {
    let result = sqlx::query(
        "UPDATE sandboxes SET proxy_id = $3, updated_at = now()
          WHERE environment_id = $1 AND id = $2",
    )
    .bind(scope.environment_id())
    .bind(sandbox_id)
    .bind(proxy_id)
    .execute(scope.connection())
    .await?;
    if result.rows_affected() == 0 {
        return Err(StoreError::NotFound);
    }
    Ok(())
}

/// Point a provider profile at an exit.
pub async fn set_provider_proxy(
    scope: &mut TenantScope<'_>,
    provider_id: Uuid,
    proxy_id: Option<Uuid>,
) -> Result<(), StoreError> {
    let result = sqlx::query(
        "UPDATE provider_profiles SET proxy_id = $3, version = version + 1, updated_at = now()
          WHERE environment_id = $1 AND id = $2",
    )
    .bind(scope.environment_id())
    .bind(provider_id)
    .bind(proxy_id)
    .execute(scope.connection())
    .await?;
    if result.rows_affected() == 0 {
        return Err(StoreError::NotFound);
    }
    Ok(())
}

/// The exit a provider profile is called through, when one is chosen.
pub async fn for_provider(
    scope: &mut TenantScope<'_>,
    provider_id: Uuid,
) -> Result<Option<Proxy>, StoreError> {
    let sql = format!(
        "SELECT {COLUMNS} FROM proxies
          WHERE environment_id = $1
            AND id = (SELECT proxy_id FROM provider_profiles WHERE id = $2 AND environment_id = $1)"
    );
    let row = sqlx::query(&sql)
        .bind(scope.environment_id())
        .bind(provider_id)
        .fetch_optional(scope.connection())
        .await?;
    row.map(|row| row_to_proxy(&row)).transpose()
}
