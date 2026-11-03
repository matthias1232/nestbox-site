//! Who is connected to a sandbox right now (ADR 0055).
//!
//! Presence is read from the session rows, so the owner and an admin see the
//! same list: terminals, SSH logins and desktop viewers, with the tab each is
//! in. Closing a tab for everyone is filled by package B14.

use uuid::Uuid;

use super::{require, ServiceContext};
use crate::domain::auth::{Permission, Principal};
use crate::domain::presence::PresenceEntry;
use crate::error::ApiError;
use crate::store::TenantScope;

/// How long an open row counts as present without a fresh heartbeat (seconds).
const PRESENT_SECONDS: i64 = 35 * 60;

/// Everyone connected to this sandbox now.
pub async fn list(
    ctx: &ServiceContext,
    principal: &Principal,
    sandbox_id: Uuid,
) -> Result<Vec<PresenceEntry>, ApiError> {
    require(principal, Permission::SandboxRead)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    crate::store::postgres::sandboxes::get(&mut scope, sandbox_id)
        .await
        .map_err(ApiError::from)?;
    let entries = crate::store::postgres::presence::list_open(&mut scope, sandbox_id, PRESENT_SECONDS)
        .await
        .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(entries)
}

/// Tell every open console that the people in this sandbox changed.
pub fn announce(ctx: &ServiceContext, environment_id: Uuid, sandbox_id: Uuid) {
    ctx.bus.publish(crate::domain::telemetry::LiveEvent::SandboxControlChanged {
        sandbox_id: Some(sandbox_id),
        environment_id,
        what: "presence".to_string(),
        detail: serde_json::json!({}),
    });
}

/// Close one terminal tab for everyone in the sandbox (filled by package B14).
pub async fn close_tab_for_all(
    ctx: &ServiceContext,
    principal: &Principal,
    sandbox_id: Uuid,
    tab_id: &str,
) -> Result<(), ApiError> {
    require(principal, Permission::SandboxExec)?;
    if tab_id.is_empty()
        || tab_id.len() > 64
        || !tab_id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(ApiError::validation("A tab id is 1 to 64 characters of A-Z, 0-9, _ and -."));
    }
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let sandbox = crate::store::postgres::sandboxes::get(&mut scope, sandbox_id)
        .await
        .map_err(ApiError::from)?;
    if tab_id != "ssh" {
        let command = vec![
            "sh".to_string(),
            "-c".to_string(),
            "tmux kill-session -t \"nestbox-$1\" 2>/dev/null || true".to_string(),
            "sh".to_string(),
            tab_id.to_string(),
        ];
        if let Err(error) = ctx.runtime.exec(&sandbox.name, &command, 15).await {
            tracing::warn!(%sandbox_id, %tab_id, error = %error, "tmux session not ended");
        }
    }
    crate::store::postgres::terminal_tabs::delete_for_all(&mut scope, sandbox_id, tab_id)
        .await
        .map_err(ApiError::from)?;
    crate::store::postgres::audit::record(
        &mut scope,
        crate::store::postgres::audit::NewAuditEvent {
            actor_id: Some(principal.user_id),
            actor_label: &principal.display_name,
            kind: "sandbox.terminal_tab_closed",
            subject_type: Some("sandbox"),
            subject_id: Some(sandbox_id),
            detail: &serde_json::json!({ "tab": tab_id }),
            request_id: None,
        },
    )
    .await
    .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    announce(ctx, principal.environment_id, sandbox_id);
    Ok(())
}
