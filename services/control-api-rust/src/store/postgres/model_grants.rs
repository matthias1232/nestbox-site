//! Rows of `model_grants`: the models an environment, a user or one sandbox may
//! use (ADR 0056).
//!
//! The inheritance rule itself lives in `domain::model_grant::effective`; every
//! read here returns the rows that can apply to one sandbox and nothing more, so
//! the rule cannot be widened by a query that forgot a filter.

use sqlx::Row;
use uuid::Uuid;

use super::super::TenantScope;
use crate::domain::model_grant::{ModelGrant, NewModelGrant};
use crate::error::StoreError;

/// The columns of one grant, named so a schema change fails here rather than in
/// a mapping that guessed.
const GRANT_COLUMNS: &str =
    "id, scope, user_id, sandbox_id, model_name, provider_profile_id, upstream_model, \
     is_default, keep_warm, created_at";

/// Map one row to a grant.
fn row_to_grant(row: &sqlx::postgres::PgRow) -> Result<ModelGrant, StoreError> {
    Ok(ModelGrant {
        id: row.try_get("id")?,
        scope: row.try_get("scope")?,
        user_id: row.try_get("user_id")?,
        sandbox_id: row.try_get("sandbox_id")?,
        model_name: row.try_get("model_name")?,
        provider_profile_id: row.try_get("provider_profile_id")?,
        upstream_model: row.try_get("upstream_model")?,
        is_default: row.try_get("is_default")?,
        keep_warm: row.try_get("keep_warm")?,
        created_at: row.try_get("created_at")?,
    })
}

/// Grants of this environment, optionally narrowed to one sandbox or one user.
///
/// Both filters together are the caller's question ("what applies to this
/// sandbox's owner"), not a contradiction: a grant matches either filter.
pub async fn list(
    scope: &mut TenantScope<'_>,
    sandbox_id: Option<Uuid>,
    user_id: Option<Uuid>,
) -> Result<Vec<ModelGrant>, StoreError> {
    let sql = format!(
        "SELECT {GRANT_COLUMNS} FROM model_grants
          WHERE environment_id = $1
            AND ($2::uuid IS NULL OR sandbox_id = $2::uuid)
            AND ($3::uuid IS NULL OR user_id = $3::uuid)
          ORDER BY scope, model_name"
    );
    let rows = sqlx::query(&sql)
        .bind(scope.environment_id())
        .bind(sandbox_id)
        .bind(user_id)
        .fetch_all(scope.connection())
        .await?;
    rows.iter().map(row_to_grant).collect()
}

/// One grant, or `None` when it does not exist in this environment.
pub async fn get(
    scope: &mut TenantScope<'_>,
    id: Uuid,
) -> Result<Option<ModelGrant>, StoreError> {
    let sql = format!(
        "SELECT {GRANT_COLUMNS} FROM model_grants WHERE environment_id = $1 AND id = $2"
    );
    let row = sqlx::query(&sql)
        .bind(scope.environment_id())
        .bind(id)
        .fetch_optional(scope.connection())
        .await?;
    match row {
        Some(row) => Ok(Some(row_to_grant(&row)?)),
        None => Ok(None),
    }
}

/// Every grant that can apply to one sandbox: the environment's, its owner's and
/// its own.
pub async fn for_sandbox(
    scope: &mut TenantScope<'_>,
    sandbox_id: Uuid,
    owner_user_id: Option<Uuid>,
) -> Result<Vec<ModelGrant>, StoreError> {
    let sql = format!(
        "SELECT {GRANT_COLUMNS} FROM model_grants
          WHERE environment_id = $1
            AND (scope = 'environment'
                 OR (scope = 'sandbox' AND sandbox_id = $2)
                 OR (scope = 'user' AND user_id = $3::uuid))
          ORDER BY model_name"
    );
    let rows = sqlx::query(&sql)
        .bind(scope.environment_id())
        .bind(sandbox_id)
        .bind(owner_user_id)
        .fetch_all(scope.connection())
        .await?;
    rows.iter().map(row_to_grant).collect()
}

/// Store one grant and return it as the database holds it.
///
/// The unique index on (scope, user, sandbox, model name) is what makes a
/// duplicate a 409 instead of a second row that would shadow the first.
pub async fn insert(
    scope: &mut TenantScope<'_>,
    new: &NewModelGrant,
    created_by: Option<Uuid>,
) -> Result<ModelGrant, StoreError> {
    let sql = format!(
        "INSERT INTO model_grants
            (environment_id, scope, user_id, sandbox_id, model_name, provider_profile_id,
             upstream_model, is_default, keep_warm, created_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
         RETURNING {GRANT_COLUMNS}"
    );
    let row = sqlx::query(&sql)
        .bind(scope.environment_id())
        .bind(&new.scope)
        .bind(new.user_id)
        .bind(new.sandbox_id)
        .bind(&new.model_name)
        .bind(new.provider_profile_id)
        .bind(&new.upstream_model)
        .bind(new.is_default)
        .bind(new.keep_warm)
        .bind(created_by)
        .fetch_one(scope.connection())
        .await?;
    row_to_grant(&row)
}

/// Remove one grant, returning the row that was removed.
pub async fn delete(
    scope: &mut TenantScope<'_>,
    id: Uuid,
) -> Result<Option<ModelGrant>, StoreError> {
    let sql = format!(
        "DELETE FROM model_grants WHERE environment_id = $1 AND id = $2 RETURNING {GRANT_COLUMNS}"
    );
    let row = sqlx::query(&sql)
        .bind(scope.environment_id())
        .bind(id)
        .fetch_optional(scope.connection())
        .await?;
    match row {
        Some(row) => Ok(Some(row_to_grant(&row)?)),
        None => Ok(None),
    }
}

/// The sandboxes a grant change reaches: an environment grant reaches all of
/// them, a user grant the sandboxes that user owns, a sandbox grant one.
pub async fn sandboxes_affected(
    scope: &mut TenantScope<'_>,
    grant: &ModelGrant,
) -> Result<Vec<Uuid>, StoreError> {
    let ids: Vec<Uuid> = match grant.scope.as_str() {
        "sandbox" => grant.sandbox_id.into_iter().collect(),
        "user" => {
            sqlx::query_scalar(
                "SELECT id FROM sandboxes
                  WHERE environment_id = $1 AND owner_user_id = $2::uuid AND status <> 'deleted'",
            )
            .bind(scope.environment_id())
            .bind(grant.user_id)
            .fetch_all(scope.connection())
            .await?
        }
        _ => {
            sqlx::query_scalar(
                "SELECT id FROM sandboxes WHERE environment_id = $1 AND status <> 'deleted'",
            )
            .bind(scope.environment_id())
            .fetch_all(scope.connection())
            .await?
        }
    };
    Ok(ids)
}

/// The facts of one sandbox that a grant change needs, and nothing else.
#[derive(Debug, Clone)]
pub struct SandboxRef {
    /// The sandbox.
    pub id: Uuid,
    /// Its owner, whose user grants apply to it.
    pub owner_user_id: Option<Uuid>,
    /// The pack, which decides whether an agent has to be restarted.
    pub pack_key: String,
    /// The lifecycle status; only a running sandbox has files to rewrite.
    pub status: String,
}

/// Map one sandbox row to the facts a grant change needs.
fn row_to_ref(row: &sqlx::postgres::PgRow) -> Result<SandboxRef, StoreError> {
    Ok(SandboxRef {
        id: row.try_get("id")?,
        owner_user_id: row.try_get("owner_user_id")?,
        pack_key: row.try_get("pack_key")?,
        status: row.try_get("status")?,
    })
}

/// Read one sandbox's facts inside this scope.
///
/// Row-level security decides whether the caller may see the row at all, which
/// is why the read is scoped rather than unconditional.
pub async fn sandbox_ref(
    scope: &mut TenantScope<'_>,
    sandbox_id: Uuid,
) -> Result<Option<SandboxRef>, StoreError> {
    let row = sqlx::query(
        "SELECT id, owner_user_id, pack_key, status FROM sandboxes
          WHERE environment_id = $1 AND id = $2",
    )
    .bind(scope.environment_id())
    .bind(sandbox_id)
    .fetch_optional(scope.connection())
    .await?;
    match row {
        Some(row) => Ok(Some(row_to_ref(&row)?)),
        None => Ok(None),
    }
}

/// Read one sandbox's facts by name, for the paths that name a sandbox instead of
/// carrying its id (the accounting proxy's router).
pub async fn sandbox_ref_by_name(
    scope: &mut TenantScope<'_>,
    name: &str,
) -> Result<Option<SandboxRef>, StoreError> {
    let row = sqlx::query(
        "SELECT id, owner_user_id, pack_key, status FROM sandboxes
          WHERE environment_id = $1 AND name = $2",
    )
    .bind(scope.environment_id())
    .bind(name)
    .fetch_optional(scope.connection())
    .await?;
    match row {
        Some(row) => Ok(Some(row_to_ref(&row)?)),
        None => Ok(None),
    }
}
