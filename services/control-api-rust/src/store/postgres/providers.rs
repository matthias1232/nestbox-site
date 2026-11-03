//! Provider profiles and secret references.

use sqlx::Row;
use uuid::Uuid;

use super::super::TenantScope;
use crate::domain::provider::{ProviderKind, ProviderProfile, ProviderStatus, SecretKind, SecretReference};
use crate::error::StoreError;

const PROFILE_COLUMNS: &str = "id, environment_id, name, kind, endpoint, model, secret_ref, \
     credential_expiry, labels, version, created_at, updated_at, status, node_id, \
     model_install_id, last_probed_at, proxy_id";

fn row_to_profile(row: &sqlx::postgres::PgRow) -> Result<ProviderProfile, StoreError> {
    let kind_raw: String = row.try_get("kind")?;
    let secret_ref: Option<String> = row.try_get("secret_ref")?;
    let status_raw: String = row.try_get("status")?;
    Ok(ProviderProfile {
        id: row.try_get("id")?,
        environment_id: row.try_get("environment_id")?,
        name: row.try_get("name")?,
        kind: ProviderKind::parse(&kind_raw).unwrap_or(ProviderKind::OpenaiCompatible),
        endpoint: row.try_get("endpoint")?,
        model: row.try_get("model")?,
        // The response reports whether a credential is configured, never a
        // value. A locator present means configured.
        credential_configured: secret_ref.is_some(),
        secret_ref,
        credential_expiry: row.try_get("credential_expiry")?,
        labels: row.try_get("labels")?,
        version: row.try_get("version")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
        status: ProviderStatus::parse(&status_raw).unwrap_or(ProviderStatus::Reachable),
        node_id: row.try_get("node_id")?,
        model_install_id: row.try_get("model_install_id")?,
        last_probed_at: row.try_get("last_probed_at")?,
        proxy_id: row.try_get("proxy_id")?,
    })
}

/// All provider profiles in this environment.
pub async fn list(scope: &mut TenantScope<'_>) -> Result<Vec<ProviderProfile>, StoreError> {
    let sql = format!(
        "SELECT {PROFILE_COLUMNS} FROM provider_profiles
          WHERE environment_id = $1 ORDER BY name"
    );
    let rows = sqlx::query(&sql)
        .bind(scope.environment_id())
        .fetch_all(scope.connection())
        .await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(row_to_profile(&row)?);
    }
    Ok(out)
}

/// One provider profile.
pub async fn get(scope: &mut TenantScope<'_>, id: Uuid) -> Result<ProviderProfile, StoreError> {
    let sql = format!(
        "SELECT {PROFILE_COLUMNS} FROM provider_profiles
          WHERE environment_id = $1 AND id = $2"
    );
    let row = sqlx::query(&sql)
        .bind(scope.environment_id())
        .bind(id)
        .fetch_optional(scope.connection())
        .await?
        .ok_or(StoreError::NotFound)?;
    row_to_profile(&row)
}

/// Insert a provider profile.
pub struct NewProviderProfile<'a> {
    pub name: &'a str,
    pub kind: ProviderKind,
    pub endpoint: &'a str,
    pub model: Option<&'a str>,
    pub secret_ref: Option<&'a str>,
    pub labels: &'a serde_json::Value,
}

pub async fn insert(
    scope: &mut TenantScope<'_>,
    new: NewProviderProfile<'_>,
) -> Result<ProviderProfile, StoreError> {
    let sql = format!(
        "WITH inserted AS (
            INSERT INTO provider_profiles
                (environment_id, name, kind, endpoint, model, secret_ref, labels)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
         RETURNING {PROFILE_COLUMNS}
         )
         SELECT {PROFILE_COLUMNS} FROM inserted"
    );
    let row = sqlx::query(&sql)
        .bind(scope.environment_id())
        .bind(new.name)
        .bind(new.kind.as_str())
        .bind(new.endpoint)
        .bind(new.model)
        .bind(new.secret_ref)
        .bind(new.labels)
        .fetch_one(scope.connection())
        .await?;
    row_to_profile(&row)
}

/// Insert the profile an install uses, in state `pending`.
///
/// Separate from `insert` because the install path sets columns an operator
/// does not: the status (nothing answers yet), the node, and the install the
/// profile belongs to. The status column arrives in 0016 with a default of
/// `reachable` for existing cloud profiles; this insert overrides it.
pub async fn insert_pending_for_install(
    scope: &mut TenantScope<'_>,
    name: &str,
    endpoint: &str,
    model: Option<&str>,
    node_id: Uuid,
    model_install_id: Uuid,
) -> Result<ProviderProfile, StoreError> {
    let sql = format!(
        "WITH inserted AS (
            INSERT INTO provider_profiles
                (environment_id, name, kind, endpoint, model, status, node_id,
                 model_install_id, last_probed_at)
            VALUES ($1, $2, 'local_openai_compatible', $3, $4, 'pending', $5, $6, NULL)
         RETURNING {PROFILE_COLUMNS}
         )
         SELECT {PROFILE_COLUMNS} FROM inserted"
    );
    let row = sqlx::query(&sql)
        .bind(scope.environment_id())
        .bind(name)
        .bind(endpoint)
        .bind(model)
        .bind(node_id)
        .bind(model_install_id)
        .fetch_one(scope.connection())
        .await?;
    row_to_profile(&row)
}

/// Record what the last probe found.
///
/// The status is the only field a probe changes, and it changes it for every
/// caller: the inference environment filters on it, so a stale status is a
/// sandbox pointed at an endpoint that no longer answers.
pub async fn set_status(
    scope: &mut TenantScope<'_>,
    id: Uuid,
    status: ProviderStatus,
) -> Result<(), StoreError> {
    let affected = sqlx::query(
        "UPDATE provider_profiles
            SET status = $3, last_probed_at = now(), updated_at = now()
          WHERE environment_id = $1 AND id = $2",
    )
    .bind(scope.environment_id())
    .bind(id)
    .bind(status.as_str())
    .execute(scope.connection())
    .await?
    .rows_affected();
    if affected == 0 {
        return Err(StoreError::NotFound);
    }
    Ok(())
}

/// Point an install's profile at the address its server is really published on.
///
/// The profile is written when the install starts, before any container exists,
/// and on a joined node the port is chosen only when the server starts. This is
/// the correction step: until it runs, the row holds a placeholder, and every
/// sandbox that selects the profile would be pointed at an address nobody
/// answers on. Only the endpoint moves; the status stays what the probe found.
pub async fn set_endpoint(
    scope: &mut TenantScope<'_>,
    id: Uuid,
    endpoint: &str,
) -> Result<(), StoreError> {
    let affected = sqlx::query(
        "UPDATE provider_profiles
            SET endpoint = $3, version = version + 1, updated_at = now()
          WHERE environment_id = $1 AND id = $2",
    )
    .bind(scope.environment_id())
    .bind(id)
    .bind(endpoint)
    .execute(scope.connection())
    .await?
    .rows_affected();
    if affected == 0 {
        return Err(StoreError::NotFound);
    }
    Ok(())
}

/// Names of the sandboxes that select this profile.
///
/// A profile a sandbox still points at is not deleted quietly: the sandbox
/// would start with an environment that names an endpoint nobody maintains.
pub async fn sandboxes_using(
    scope: &mut TenantScope<'_>,
    profile_id: Uuid,
) -> Result<Vec<String>, StoreError> {
    let rows = sqlx::query(
        "SELECT name FROM sandboxes
          WHERE environment_id = $1 AND provider_profile_id = $2 AND status <> 'deleted'
          ORDER BY name",
    )
    .bind(scope.environment_id())
    .bind(profile_id)
    .fetch_all(scope.connection())
    .await?;
    rows.iter().map(|row| Ok(row.try_get("name")?)).collect()
}

/// Delete a provider profile.
pub async fn delete(scope: &mut TenantScope<'_>, id: Uuid) -> Result<(), StoreError> {
    let affected = sqlx::query(
        "DELETE FROM provider_profiles WHERE environment_id = $1 AND id = $2",
    )
    .bind(scope.environment_id())
    .bind(id)
    .execute(scope.connection())
    .await?
    .rows_affected();
    if affected == 0 {
        Err(StoreError::NotFound)
    } else {
        Ok(())
    }
}

/// Replace a provider profile's mutable fields.
///
/// Name and kind are not mutable: a name is how other rows refer to the profile
/// in audit output, and changing the kind would silently reinterpret an existing
/// endpoint. A correction of either is a delete and a create, which is visible
/// in the audit trail.
pub async fn update(
    scope: &mut TenantScope<'_>,
    id: Uuid,
    expected_version: i64,
    endpoint: &str,
    model: Option<&str>,
    secret_ref: Option<&str>,
    labels: &serde_json::Value,
) -> Result<ProviderProfile, StoreError> {
    let sql = format!(
        "WITH updated AS (
            UPDATE provider_profiles
               SET endpoint = $3,
                   model = $4,
                   secret_ref = $5,
                   labels = $6,
                   version = version + 1,
                   updated_at = now()
             WHERE environment_id = $1 AND id = $2 AND version = $7
         RETURNING {PROFILE_COLUMNS}
         )
         SELECT {PROFILE_COLUMNS} FROM updated"
    );
    let row = sqlx::query(&sql)
        .bind(scope.environment_id())
        .bind(id)
        .bind(endpoint)
        .bind(model)
        .bind(secret_ref)
        .bind(labels)
        .bind(expected_version)
        .fetch_optional(scope.connection())
        .await?
        .ok_or(StoreError::VersionConflict)?;
    row_to_profile(&row)
}

/// Record that a credential was rotated now.
pub async fn mark_rotated(
    scope: &mut TenantScope<'_>,
    name: &str,
    expires_at: Option<chrono::DateTime<chrono::Utc>>,
) -> Result<(), StoreError> {
    sqlx::query(
        "UPDATE provider_profiles
            SET credential_expiry = $3, version = version + 1, updated_at = now()
          WHERE environment_id = $1 AND name = $2",
    )
    .bind(scope.environment_id())
    .bind(name)
    .bind(expires_at)
    .execute(scope.connection())
    .await?;
    Ok(())
}

/// All secret references. Locators only; there is no value column to select.
pub async fn list_secrets(scope: &mut TenantScope<'_>) -> Result<Vec<SecretReference>, StoreError> {
    let rows = sqlx::query(
        "SELECT id, environment_id, name, locator, kind, rotated_at
           FROM secret_references WHERE environment_id = $1 ORDER BY name",
    )
    .bind(scope.environment_id())
    .fetch_all(scope.connection())
    .await?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let kind_raw: String = row.try_get("kind")?;
        out.push(SecretReference {
            id: row.try_get("id")?,
            environment_id: row.try_get("environment_id")?,
            name: row.try_get("name")?,
            locator: row.try_get("locator")?,
            kind: SecretKind::parse(&kind_raw).unwrap_or(SecretKind::ApiKey),
            rotated_at: row.try_get("rotated_at")?,
        });
    }
    Ok(out)
}

/// Register a secret locator.
pub async fn insert_secret(
    scope: &mut TenantScope<'_>,
    name: &str,
    locator: &str,
    kind: SecretKind,
) -> Result<Uuid, StoreError> {
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO secret_references (environment_id, name, locator, kind)
         VALUES ($1, $2, $3, $4) RETURNING id",
    )
    .bind(scope.environment_id())
    .bind(name)
    .bind(locator)
    .bind(kind.as_str())
    .fetch_one(scope.connection())
    .await?;
    Ok(id)
}
