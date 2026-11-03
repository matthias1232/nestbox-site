//! The one call the Workbox page needs: everything about a sandbox's controls,
//! and the cheap summary the admin list shows for every sandbox (ADR 0053-0057).
//!
//! The summary never fails because one part fails: an agent list that cannot be
//! read (a stopped sandbox) becomes an empty list and one entry under `errors`.
//! A page that shows seven of eight things is more useful than an error page.

use chrono::{DateTime, Utc};
use uuid::Uuid;

use super::{require, ServiceContext};
use crate::domain::auth::{Permission, Principal};
use crate::error::ApiError;
use crate::store::TenantScope;

/// The latest expiry of the windows that allow everything now.
///
/// `None` means there is no limit: either no window is open, or one of them is
/// open-ended, and an open-ended window does not end by itself.
fn allow_all_until(expiries: &[Option<DateTime<Utc>>]) -> Option<DateTime<Utc>> {
    if expiries.is_empty() || expiries.iter().any(Option::is_none) {
        return None;
    }
    expiries.iter().flatten().copied().max()
}

/// Which recording level a sandbox's two mutes mean at `now`, and when it ends.
///
/// The flow mute (`telemetry_mute_until`) wins: while it runs nothing is stored, so the
/// level is `off`. Only the body mute (`telemetry_mute_bodies_until`) means `flows`: the
/// flows are stored, bodies and chats are not. Both in the past or absent means `full`.
/// The `until` is the end of the decisive mute, so the strip can say how long it lasts.
fn recording_state(
    now: DateTime<Utc>,
    mute_until: Option<DateTime<Utc>>,
    bodies_until: Option<DateTime<Utc>>,
) -> (&'static str, Option<DateTime<Utc>>) {
    let running = |end: Option<DateTime<Utc>>| end.filter(|value| *value > now);
    if let Some(until) = running(mute_until) {
        return ("off", Some(until));
    }
    if let Some(until) = running(bodies_until) {
        return ("flows", Some(until));
    }
    ("full", None)
}

/// Everything the Workbox shows for one sandbox.
pub async fn summary(
    ctx: &ServiceContext,
    principal: &Principal,
    sandbox_id: Uuid,
) -> Result<serde_json::Value, ApiError> {
    require(principal, Permission::SandboxRead)?;

    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let sandbox = crate::store::postgres::sandboxes::get(&mut scope, sandbox_id)
        .await
        .map_err(ApiError::from)?;
    let node = crate::store::postgres::control::node_brief(&mut scope, sandbox.node_id)
        .await
        .map_err(ApiError::from)?;
    // The recording mutes live on the sandbox row (B11), so read them in the same scope.
    let (mute_until, mute_bodies_until) =
        crate::store::postgres::sandboxes::telemetry_mute(&mut scope, sandbox_id)
            .await
            .map_err(ApiError::from)?;
    // Only the windows that cover this sandbox: its own and the environment's.
    let windows: Vec<serde_json::Value> = crate::store::postgres::policies::list_windows(&mut scope)
        .await
        .map_err(ApiError::from)?
        .into_iter()
        .filter(|window| window.sandbox_id.is_none() || window.sandbox_id == Some(sandbox_id))
        .map(|window| {
            serde_json::json!({
                "id": window.id,
                "sandbox_id": window.sandbox_id,
                "scope": window.scope,
                "mode": window.mode,
                "channel": window.channel,
                "reason": window.reason,
                "expires_at": window.expires_at,
            })
        })
        .collect();
    scope.commit().await.map_err(ApiError::from)?;

    let mut errors = serde_json::Map::new();
    let (agents, presence, models) = tokio::join!(
        super::agent_auth_service::list(ctx, principal, sandbox_id),
        super::presence_service::list(ctx, principal, sandbox_id),
        super::model_grant_service::effective_for_sandbox(ctx, principal, sandbox_id),
    );
    let agents = agents.unwrap_or_else(|error| {
        errors.insert("agents".to_string(), error.message.clone().into());
        Vec::new()
    });
    let presence = presence.unwrap_or_else(|error| {
        errors.insert("presence".to_string(), error.message.clone().into());
        Vec::new()
    });
    let models = models.unwrap_or_else(|error| {
        errors.insert("models".to_string(), error.message.clone().into());
        Vec::new()
    });

    let egress = super::policy_service::egress_mode(ctx, principal, sandbox_id).await?;
    let expiries: Vec<Option<DateTime<Utc>>> = windows
        .iter()
        .map(|window| {
            window
                .get("expires_at")
                .and_then(|value| value.as_str())
                .and_then(|value| value.parse::<DateTime<Utc>>().ok())
        })
        .collect();
    let allow_all = egress.mode.allows_everything() || !windows.is_empty();
    let (recording_mode, recording_until) =
        recording_state(Utc::now(), mute_until, mute_bodies_until);

    Ok(serde_json::json!({
        "sandbox_id": sandbox.id,
        "name": sandbox.name,
        "status": sandbox.status.as_str(),
        "node": node.map(|(name, tunnel_state, endpoint)| serde_json::json!({
            "name": name,
            "tunnel_state": tunnel_state,
            "endpoint": endpoint,
        })),
        "egress": egress,
        "windows": windows,
        "allow_all": allow_all,
        "allow_all_until": if allow_all { allow_all_until(&expiries) } else { None },
        "recording": {
            "mode": recording_mode,
            "until": recording_until,
        },
        "agents": agents,
        "presence": presence,
        "models": models,
        "move": serde_json::Value::Null,
        "errors": serde_json::Value::Object(errors),
    }))
}

/// The per-sandbox summary the admin list shows, for every visible sandbox.
pub async fn overview(
    ctx: &ServiceContext,
    principal: &Principal,
) -> Result<serde_json::Value, ApiError> {
    require(principal, Permission::SandboxRead)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let rows = crate::store::postgres::control::overview(&mut scope)
        .await
        .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[test]
    fn an_open_ended_window_has_no_end() {
        let now = Utc::now();
        assert_eq!(allow_all_until(&[Some(now + Duration::minutes(5)), None]), None);
        assert_eq!(allow_all_until(&[]), None);
    }

    #[test]
    fn the_latest_expiry_wins() {
        let now = Utc::now();
        let latest = now + Duration::minutes(30);
        assert_eq!(
            allow_all_until(&[Some(now + Duration::minutes(5)), Some(latest)]),
            Some(latest)
        );
    }

    #[test]
    fn a_running_flow_mute_is_off_and_names_its_end() {
        let now = Utc::now();
        let until = now + Duration::hours(24);
        assert_eq!(
            recording_state(now, Some(until), Some(until)),
            ("off", Some(until))
        );
        // The flow mute alone is enough: nothing is stored, bodies or not.
        assert_eq!(
            recording_state(now, Some(until), None),
            ("off", Some(until))
        );
    }

    #[test]
    fn only_the_body_mute_means_flows() {
        let now = Utc::now();
        let until = now + Duration::hours(1);
        assert_eq!(
            recording_state(now, None, Some(until)),
            ("flows", Some(until))
        );
    }

    #[test]
    fn mutes_in_the_past_record_everything_again() {
        let now = Utc::now();
        let past = now - Duration::minutes(1);
        assert_eq!(recording_state(now, Some(past), Some(past)), ("full", None));
        assert_eq!(recording_state(now, None, None), ("full", None));
    }
}
