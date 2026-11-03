//! Switching a sandbox's egress mode while it runs (ADR 0052).
//!
//! The proxy already decides per CONNECT which mode applies, so a change needs
//! no new container: it is stored, the OpenShell policy is pushed, and the
//! console is told. Only `direct` still changes the network itself and therefore
//! keeps the old rebuild path. The same file owns the revert timer: `apply` arms
//! it, `revert_due` fires it from the reconcile loop.

use chrono::{DateTime, Utc};
use uuid::Uuid;

use super::ServiceContext;
use crate::domain::auth::Principal;
use crate::domain::policy::{EgressMode, EgressModeSetting};
use crate::error::ApiError;
use crate::store::postgres::egress_live as store;
use crate::store::TenantScope;

/// What a revert timer of zero or more than a week is told.
const REVERT_RANGE_MESSAGE: &str = "A revert timer is 1 to 10080 minutes.";

/// The timer a mode change arms: when it switches back, and to which mode.
///
/// Pure, so the range check and the arithmetic are testable without a database or
/// a clock. `None` means "no timer"; a duration outside one minute to one week is
/// refused rather than clamped, because a silently changed timer would be a promise
/// the operator did not make.
fn revert_plan(
    previous: EgressMode,
    minutes: Option<i32>,
    now: DateTime<Utc>,
) -> Result<(Option<DateTime<Utc>>, Option<&'static str>), String> {
    let Some(minutes) = minutes else {
        return Ok((None, None));
    };
    if !(1..=10_080).contains(&minutes) {
        return Err(REVERT_RANGE_MESSAGE.to_string());
    }
    Ok((
        Some(now + chrono::Duration::minutes(minutes as i64)),
        Some(previous.as_str()),
    ))
}

/// Change one sandbox's egress mode, live where that is possible.
///
/// `transparent`, `hybrid`, `forward_proxy` and `off_allow_all` are stored and take
/// effect for the next connection: the proxy and the OpenShell supervisor read the
/// mode per request, so nothing is recreated and no sandbox loses its files. The CA
/// every TLS-reading mode needs is mounted for every proxied mode now, so no trust
/// reference is passed into the store any more. A change to or from `direct` keeps
/// the old path, because `direct` changes how the container is attached to the
/// network and which exit proxy it uses.
///
/// `revert_after_minutes` arms a timer that switches back to the mode the sandbox
/// had before; `revert_due` in the reconcile loop fires it.
pub async fn apply(
    ctx: &ServiceContext,
    principal: &Principal,
    sandbox_id: Uuid,
    mode: EgressMode,
    revert_after_minutes: Option<i32>,
) -> Result<EgressModeSetting, ApiError> {
    if let Some(minutes) = revert_after_minutes {
        if !(1..=10_080).contains(&minutes) {
            return Err(ApiError::validation(REVERT_RANGE_MESSAGE));
        }
    }
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    super::policy_service::require_sandbox_scope(&mut scope, principal, Some(sandbox_id)).await?;
    let sandbox = crate::store::postgres::sandboxes::get(&mut scope, sandbox_id)
        .await
        .map_err(ApiError::from)?;
    let previous = crate::store::postgres::policies::egress_mode_for_sandbox(&mut scope, sandbox_id)
        .await
        .map_err(ApiError::from)?
        .map(|setting| setting.mode)
        .unwrap_or(EgressMode::ForwardProxy);

    // With the gateway as the default runtime, a mode decides *where* the sandbox
    // runs: reading TLS needs Docker (the gateway refuses a CONNECT to another
    // proxy), so crossing between `forward_proxy` and a reading mode also crosses
    // runtimes. That is a rebuild, not a live switch. On a Docker deployment the
    // mode stays live.
    let crosses_runtime = crate::services::sandbox_service::openshell_runtime() && mode != previous;
    // `direct` is the one mode that changes how the container is attached to the
    // network and which exit proxy it uses; that cannot be done live either.
    if crosses_runtime || mode == EgressMode::Direct || previous == EgressMode::Direct {
        scope.commit().await.map_err(ApiError::from)?;
        return super::policy_service::set_egress_mode_rebuilding(
            ctx, principal, sandbox_id, mode, None,
        )
        .await;
    }

    let (revert_at, revert_to) = revert_plan(previous, revert_after_minutes, Utc::now())
        .map_err(ApiError::validation)?;
    let setting = crate::store::postgres::policies::set_egress_mode(
        &mut scope,
        sandbox_id,
        mode,
        None,
        true,
        revert_at,
        revert_to,
    )
    .await
    .map_err(ApiError::from)?;

    crate::store::postgres::audit::record(
        &mut scope,
        crate::store::postgres::audit::NewAuditEvent {
            actor_id: Some(principal.user_id),
            actor_label: &principal.display_name,
            kind: "sandbox.egress_mode",
            subject_type: Some("sandbox"),
            subject_id: Some(sandbox_id),
            detail: &serde_json::json!({
                "from": previous.as_str(),
                "to": mode.as_str(),
                "revert_at": revert_at,
            }),
            request_id: None,
        },
    )
    .await
    .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;

    // A sandbox without the CA mounted (created before the CA was mounted for
    // every proxied mode) needs one recreate before TLS can be opened.
    let mut setting = setting;
    if sandbox.status == crate::domain::sandbox::SandboxStatus::Running && mode.inspects_tls() {
        let probe = vec![
            "sh".to_string(),
            "-c".to_string(),
            "test -s /run/secrets/egress-ca.crt".to_string(),
        ];
        setting.needs_recreate = match ctx.runtime.exec(&sandbox.name, &probe, 5).await {
            Ok(result) => result.exit_code != Some(0),
            Err(_) => false,
        };
    }

    spawn_openshell_sync(ctx, "after a mode change");

    ctx.bus.publish(crate::domain::telemetry::LiveEvent::SandboxControlChanged {
        sandbox_id: Some(sandbox_id),
        environment_id: sandbox.environment_id,
        what: "egress_mode".to_string(),
        detail: serde_json::json!({
            "mode": mode.as_str(),
            "revert_at": revert_at,
            "needs_recreate": setting.needs_recreate,
        }),
    });
    Ok(setting)
}

/// Switch back every sandbox whose revert timer has passed.
///
/// The reconcile loop calls this every 15 s; it runs without a principal, so it walks
/// the environments and opens a system scope per tenant. One sandbox that cannot be
/// switched back is logged and skipped instead of stopping the pass, because a stuck
/// row must not keep every other timer from firing.
pub async fn revert_due(ctx: &ServiceContext) -> Result<(), ApiError> {
    let environments = store::environment_ids(&ctx.pool)
        .await
        .map_err(ApiError::from)?;
    let mut reverted = false;
    for environment_id in environments {
        // The read commits on its own so the write below starts with a clean scope.
        let due_rows = {
            let mut scope = TenantScope::begin(&ctx.pool, environment_id)
                .await
                .map_err(ApiError::from)?;
            let rows = store::due(&mut scope).await.map_err(ApiError::from)?;
            scope.commit().await.map_err(ApiError::from)?;
            rows
        };
        for (sandbox_id, revert_to) in due_rows {
            match revert_one(ctx, environment_id, sandbox_id, &revert_to).await {
                Ok(true) => reverted = true,
                Ok(false) => {}
                Err(error) => tracing::warn!(
                    sandbox = %sandbox_id,
                    code = error.code.0,
                    "egress mode not switched back"
                ),
            }
        }
    }
    if reverted {
        spawn_openshell_sync(ctx, "after a revert");
    }
    Ok(())
}

/// Switch one sandbox back and tell the console. `false` when nothing was changed.
async fn revert_one(
    ctx: &ServiceContext,
    environment_id: Uuid,
    sandbox_id: Uuid,
    revert_to: &str,
) -> Result<bool, ApiError> {
    let mode = EgressMode::parse(revert_to).unwrap_or(EgressMode::ForwardProxy);
    if mode == EgressMode::Direct {
        // `direct` needs a new container, and a timer never arms it; a hand-edited
        // row saying so is left alone rather than rebuilt by a background pass.
        tracing::warn!(sandbox = %sandbox_id, "revert target is `direct`; skipped");
        return Ok(false);
    }
    let mut scope = TenantScope::begin(&ctx.pool, environment_id)
        .await
        .map_err(ApiError::from)?;
    let setting = crate::store::postgres::policies::set_egress_mode(
        &mut scope,
        sandbox_id,
        mode,
        None,
        true,
        None,
        None,
    )
    .await
    .map_err(ApiError::from)?;
    crate::store::postgres::audit::record(
        &mut scope,
        crate::store::postgres::audit::NewAuditEvent {
            actor_id: None,
            actor_label: "system",
            kind: "sandbox.egress_mode_reverted",
            subject_type: Some("sandbox"),
            subject_id: Some(sandbox_id),
            detail: &serde_json::json!({
                "to": mode.as_str(),
                "revert_to": revert_to,
            }),
            request_id: None,
        },
    )
    .await
    .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;

    ctx.bus.publish(crate::domain::telemetry::LiveEvent::SandboxControlChanged {
        sandbox_id: Some(sandbox_id),
        environment_id,
        what: "egress_mode".to_string(),
        detail: serde_json::json!({
            "mode": setting.mode.as_str(),
            "revert_at": serde_json::Value::Null,
            "needs_recreate": setting.needs_recreate,
        }),
    });
    Ok(true)
}

/// Push the OpenShell policies in the background: a push must not delay the answer.
fn spawn_openshell_sync(ctx: &ServiceContext, why: &'static str) {
    let ctx2 = ctx.clone();
    tokio::spawn(async move {
        if let Err(error) = super::policy_service::sync_openshell_policies(&ctx2).await {
            tracing::warn!(
                code = error.code.0,
                reason = why,
                "OpenShell policies not pushed"
            );
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-01-01T12:00:00Z")
            .map(|value| value.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now())
    }

    #[test]
    fn no_minutes_arms_no_timer() {
        assert_eq!(revert_plan(EgressMode::Hybrid, None, now()), Ok((None, None)));
    }

    #[test]
    fn thirty_minutes_switch_back_to_the_previous_mode() {
        let (revert_at, revert_to) =
            revert_plan(EgressMode::Transparent, Some(30), now()).unwrap_or((None, None));
        assert_eq!(revert_at, Some(now() + chrono::Duration::minutes(30)));
        assert_eq!(revert_to, Some("transparent"));
    }

    #[test]
    fn zero_minutes_is_refused() {
        assert!(revert_plan(EgressMode::Hybrid, Some(0), now()).is_err());
    }

    #[test]
    fn more_than_a_week_is_refused() {
        assert!(revert_plan(EgressMode::Hybrid, Some(10_081), now()).is_err());
    }
}
