//! The readiness reads: which profile serves a granted model, and on which node.
//! Filled by package B22.

use sqlx::Row;
use uuid::Uuid;

use super::super::TenantScope;
use crate::error::StoreError;

/// A model grant whose profile may be local to a node.
#[derive(Debug, Clone)]
pub struct WarmTarget {
    /// The provider profile.
    pub profile_id: Uuid,
    /// `local` for a model served on a node, anything else for an API.
    pub kind: String,
    /// The `.../v1` endpoint of the profile.
    pub endpoint: String,
    /// The model id at the provider.
    pub upstream_model: String,
    /// The node the install lives on, when it is an install.
    pub node_id: Option<Uuid>,
    /// The install state, when it is an install (`serving`, `stopped`, ...).
    pub install_state: Option<String>,
}

/// The profiles of the granted models of one sandbox.
pub async fn warm_targets(
    scope: &mut TenantScope<'_>,
    sandbox_id: Uuid,
) -> Result<Vec<WarmTarget>, StoreError> {
    let rows = sqlx::query(
        "WITH grants AS (
             SELECT DISTINCT model_name, provider_profile_id
               FROM model_grants g
              WHERE g.environment_id = $1
                AND (g.scope = 'environment'
                     OR g.sandbox_id = $2
                     OR (g.user_id IS NOT NULL AND g.user_id = (
                          SELECT owner_user_id FROM sandboxes WHERE id = $2)))
         )
         SELECT p.id AS profile_id, p.kind, p.endpoint, p.model AS profile_model,
                m.model_name, i.node_id, i.state::text AS install_state
           FROM grants m
           JOIN provider_profiles p ON p.id = m.provider_profile_id
           LEFT JOIN model_installs i ON i.provider_profile_id = p.id
          ORDER BY m.model_name",
    )
    .bind(scope.environment_id())
    .bind(sandbox_id)
    .fetch_all(scope.connection())
    .await?;
    let mut out = Vec::new();
    for row in rows {
        let model_name: String = row.try_get("model_name")?;
        let profile_model: Option<String> = row.try_get("profile_model")?;
        out.push(WarmTarget {
            profile_id: row.try_get("profile_id")?,
            kind: row.try_get("kind")?,
            endpoint: row.try_get("endpoint")?,
            upstream_model: profile_model.unwrap_or(model_name),
            node_id: row.try_get("node_id")?,
            install_state: row.try_get("install_state")?,
        });
    }
    Ok(out)
}

/// One profile by id, for the on-demand warm.
pub async fn target_by_model(
    scope: &mut TenantScope<'_>,
    model: &str,
) -> Result<Option<WarmTarget>, StoreError> {
    let row = sqlx::query(
        "SELECT p.id AS profile_id, p.kind, p.endpoint, p.model AS profile_model,
                i.node_id, i.state::text AS install_state
           FROM provider_profiles p
           LEFT JOIN model_installs i ON i.provider_profile_id = p.id
          WHERE p.environment_id = $1 AND (p.model = $2 OR p.name = $2)
          ORDER BY p.created_at
          LIMIT 1",
    )
    .bind(scope.environment_id())
    .bind(model)
    .fetch_optional(scope.connection())
    .await?;
    let Some(row) = row else { return Ok(None) };
    let profile_model: Option<String> = row.try_get("profile_model")?;
    Ok(Some(WarmTarget {
        profile_id: row.try_get("profile_id")?,
        kind: row.try_get("kind")?,
        endpoint: row.try_get("endpoint")?,
        upstream_model: profile_model.unwrap_or_else(|| model.to_string()),
        node_id: row.try_get("node_id")?,
        install_state: row.try_get("install_state")?,
    }))
}

/// The profiles of every keep-warm grant of this environment.
pub async fn keep_warm(scope: &mut TenantScope<'_>) -> Result<Vec<WarmTarget>, StoreError> {
    let rows = sqlx::query(
        "SELECT DISTINCT p.id AS profile_id, p.kind, p.endpoint, p.model AS profile_model,
                g.model_name, i.node_id, i.state::text AS install_state
           FROM model_grants g
           JOIN provider_profiles p ON p.id = g.provider_profile_id
           LEFT JOIN model_installs i ON i.provider_profile_id = p.id
          WHERE g.environment_id = $1 AND g.keep_warm
          ORDER BY p.id",
    )
    .bind(scope.environment_id())
    .fetch_all(scope.connection())
    .await?;
    let mut out = Vec::new();
    for row in rows {
        let model_name: String = row.try_get("model_name")?;
        let profile_model: Option<String> = row.try_get("profile_model")?;
        out.push(WarmTarget {
            profile_id: row.try_get("profile_id")?,
            kind: row.try_get("kind")?,
            endpoint: row.try_get("endpoint")?,
            upstream_model: profile_model.unwrap_or(model_name),
            node_id: row.try_get("node_id")?,
            install_state: row.try_get("install_state")?,
        });
    }
    Ok(out)
}
