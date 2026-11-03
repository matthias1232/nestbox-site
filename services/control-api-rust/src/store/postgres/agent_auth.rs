//! Rows of `sandbox_agent_auth`: the chosen sign-in mode per agent of a sandbox.

use sqlx::Row;
use uuid::Uuid;

use super::super::TenantScope;
use crate::domain::agent_auth::{AgentAuthMode, AgentKind};
use crate::error::StoreError;

/// One stored choice.
#[derive(Debug, Clone)]
pub struct AgentAuthRow {
    /// The agent.
    pub agent: AgentKind,
    /// The mode.
    pub mode: AgentAuthMode,
    /// A restart waits for the agent to be idle.
    pub pending_restart: bool,
}

/// Every stored choice of one sandbox.
pub async fn list(scope: &mut TenantScope<'_>, sandbox_id: Uuid) -> Result<Vec<AgentAuthRow>, StoreError> {
    let rows = sqlx::query(
        "SELECT agent, mode, pending_restart FROM sandbox_agent_auth
          WHERE environment_id = $1 AND sandbox_id = $2 ORDER BY agent",
    )
    .bind(scope.environment_id())
    .bind(sandbox_id)
    .fetch_all(scope.connection())
    .await?;
    let mut out = Vec::new();
    for row in rows {
        let agent: String = row.try_get("agent")?;
        let mode: String = row.try_get("mode")?;
        let (Some(agent), Some(mode)) = (AgentKind::parse(&agent), AgentAuthMode::parse(&mode)) else {
            return Err(StoreError::UnexpectedData(format!("agent auth row {agent}/{mode}")));
        };
        out.push(AgentAuthRow {
            agent,
            mode,
            pending_restart: row.try_get("pending_restart")?,
        });
    }
    Ok(out)
}

/// Store a choice (insert or update).
pub async fn upsert(
    scope: &mut TenantScope<'_>,
    sandbox_id: Uuid,
    agent: AgentKind,
    mode: AgentAuthMode,
    pending_restart: bool,
    updated_by: Option<Uuid>,
) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO sandbox_agent_auth
            (environment_id, sandbox_id, agent, mode, pending_restart, updated_by)
         VALUES ($1, $2, $3, $4, $5, $6)
         ON CONFLICT (sandbox_id, agent) DO UPDATE
            SET mode = EXCLUDED.mode,
                pending_restart = EXCLUDED.pending_restart,
                updated_by = EXCLUDED.updated_by,
                updated_at = now(),
                version = sandbox_agent_auth.version + 1",
    )
    .bind(scope.environment_id())
    .bind(sandbox_id)
    .bind(agent.as_str())
    .bind(mode.as_str())
    .bind(pending_restart)
    .bind(updated_by)
    .execute(scope.connection())
    .await?;
    Ok(())
}

/// Set or clear the pending-restart flag of one agent.
pub async fn set_pending(
    scope: &mut TenantScope<'_>,
    sandbox_id: Uuid,
    agent: AgentKind,
    pending: bool,
) -> Result<u64, StoreError> {
    let affected = sqlx::query(
        "UPDATE sandbox_agent_auth SET pending_restart = $4, updated_at = now()
          WHERE environment_id = $1 AND sandbox_id = $2 AND agent = $3",
    )
    .bind(scope.environment_id())
    .bind(sandbox_id)
    .bind(agent.as_str())
    .bind(pending)
    .execute(scope.connection())
    .await?
    .rows_affected();
    Ok(affected)
}

/// Sandboxes of this environment with an agent waiting for a restart.
pub async fn pending(scope: &mut TenantScope<'_>) -> Result<Vec<(Uuid, AgentKind)>, StoreError> {
    let rows = sqlx::query(
        "SELECT sandbox_id, agent FROM sandbox_agent_auth
          WHERE environment_id = $1 AND pending_restart",
    )
    .bind(scope.environment_id())
    .fetch_all(scope.connection())
    .await?;
    let mut out = Vec::new();
    for row in rows {
        let agent: String = row.try_get("agent")?;
        if let Some(agent) = AgentKind::parse(&agent) {
            out.push((row.try_get("sandbox_id")?, agent));
        }
    }
    Ok(out)
}

/// The agents a pack can switch (`agent_packs.login_agents`).
pub async fn pack_agents(scope: &mut TenantScope<'_>, pack_key: &str) -> Result<Vec<AgentKind>, StoreError> {
    let value: Option<Vec<String>> =
        sqlx::query_scalar("SELECT login_agents FROM agent_packs WHERE key = $1")
            .bind(pack_key)
            .fetch_optional(scope.connection())
            .await?;
    let mut out = Vec::new();
    for agent in value.unwrap_or_default() {
        if let Some(agent) = AgentKind::parse(&agent) {
            if !out.contains(&agent) {
                out.push(agent);
            }
        }
    }
    Ok(out)
}

/// The packs of the agents a recipe sandbox was made from.
pub async fn sandbox_agent_packs(
    scope: &mut TenantScope<'_>,
    sandbox_id: Uuid,
) -> Result<Vec<String>, StoreError> {
    let packs: Vec<String> =
        sqlx::query_scalar("SELECT DISTINCT pack_key FROM sandbox_agents WHERE sandbox_id = $1")
            .bind(sandbox_id)
            .fetch_all(scope.connection())
            .await?;
    Ok(packs)
}
