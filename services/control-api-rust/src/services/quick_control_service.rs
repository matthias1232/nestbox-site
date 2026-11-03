//! One-click controls for the end user (package B11, contract C12).
//!
//! Every control is a thin mapping onto an existing service: traffic windows,
//! the egress mode with its revert timer, and the recording mutes on the sandbox.

use chrono::{Duration, Utc};
use uuid::Uuid;

use super::{ServiceContext, policy_service};
use crate::domain::auth::{Permission, Principal};
use crate::domain::policy::EgressMode;
use crate::error::ApiError;
use crate::store::TenantScope;
use crate::store::postgres::audit::{self, NewAuditEvent};
use crate::store::postgres::policies as policy_store;
use crate::store::postgres::sandboxes as sandbox_store;

/// How long a recording mute lasts when the caller gives no minutes.
const DEFAULT_MUTE_MINUTES: i64 = 24 * 60;

/// Apply one quick control and answer the sandbox's control summary (contract C12).
///
/// Validates the `what`/`value` pair first (422), then maps the control onto the
/// existing window, egress-mode or recording-mute calls, audits it as
/// `sandbox.quick` and publishes a control change.
pub async fn set(
    ctx: &ServiceContext,
    principal: &Principal,
    sandbox_id: Uuid,
    what: &str,
    value: serde_json::Value,
    minutes: Option<i32>,
) -> Result<serde_json::Value, ApiError> {
    if !principal.can(Permission::PolicyWrite) && !principal.can(Permission::SandboxPolicy) {
        return Err(ApiError::forbidden(Permission::SandboxPolicy.code()));
    }
    if let Some(minutes) = minutes {
        if !(1..=1440).contains(&minutes) {
            return Err(ApiError::validation("Minutes is 1 to 1440, or empty."));
        }
    }
    let action = parse(what, &value)?;

    // Visibility first: a sandbox the caller cannot see is a 404 before anything changes.
    {
        let mut scope = TenantScope::begin_for(&ctx.pool, principal)
            .await
            .map_err(ApiError::from)?;
        policy_service::require_sandbox_scope(&mut scope, principal, Some(sandbox_id)).await?;
        sandbox_store::get(&mut scope, sandbox_id)
            .await
            .map_err(ApiError::from)?;
        scope.commit().await.map_err(ApiError::from)?;
    }

    match action {
        Action::AllowAll(true) => {
            close_windows(ctx, principal, sandbox_id, Some("all")).await?;
            policy_service::open_window(
                ctx,
                principal,
                Some(sandbox_id),
                minutes,
                "bypass",
                "all",
                "quick: allow all",
            )
            .await?;
        }
        Action::AllowAll(false) => close_windows(ctx, principal, sandbox_id, None).await?,
        Action::Channel(channel, true) => {
            close_windows(ctx, principal, sandbox_id, Some(channel)).await?;
            policy_service::open_window(
                ctx,
                principal,
                Some(sandbox_id),
                minutes,
                "bypass",
                channel,
                &format!("quick: open {channel}"),
            )
            .await?;
        }
        Action::Channel(channel, false) => {
            close_windows(ctx, principal, sandbox_id, Some(channel)).await?
        }
        Action::Egress(mode) => {
            super::egress_live_service::apply(ctx, principal, sandbox_id, mode, minutes).await?;
        }
        Action::Recording(level) => {
            set_recording(ctx, principal, sandbox_id, level, minutes).await?;
        }
    }

    let detail = serde_json::json!({ "what": what, "value": value, "minutes": minutes });
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    audit::record(
        &mut scope,
        NewAuditEvent {
            actor_id: Some(principal.user_id),
            actor_label: &principal.display_name,
            kind: "sandbox.quick",
            subject_type: Some("sandbox"),
            subject_id: Some(sandbox_id),
            detail: &detail,
            request_id: None,
        },
    )
    .await
    .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;

    ctx.bus
        .publish(crate::domain::telemetry::LiveEvent::SandboxControlChanged {
            sandbox_id: Some(sandbox_id),
            environment_id: principal.environment_id,
            what: "quick".to_string(),
            detail,
        });

    super::control_service::summary(ctx, principal, sandbox_id).await
}

/// How much of the traffic record is kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Recording {
    Full,
    Flows,
    Off,
}

/// A validated quick control.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    AllowAll(bool),
    Channel(&'static str, bool),
    Egress(EgressMode),
    Recording(Recording),
}

/// Check a `what`/`value` pair against contract C12.
fn parse(what: &str, value: &serde_json::Value) -> Result<Action, ApiError> {
    let flag = || {
        value
            .as_bool()
            .ok_or_else(|| ApiError::validation(format!("{what} takes true or false.")))
    };
    match what {
        "allow_all" => Ok(Action::AllowAll(flag()?)),
        "dns" => Ok(Action::Channel("dns", flag()?)),
        "internet" => Ok(Action::Channel("proxy", flag()?)),
        "mcp" => Ok(Action::Channel("mcp", flag()?)),
        "egress_mode" => {
            let mode = value
                .as_str()
                .filter(|text| {
                    matches!(*text, "forward_proxy" | "transparent" | "hybrid" | "off_allow_all")
                })
                .and_then(EgressMode::parse)
                .ok_or_else(|| {
                    ApiError::validation(
                        "egress_mode is forward_proxy, transparent, hybrid or off_allow_all.",
                    )
                })?;
            Ok(Action::Egress(mode))
        }
        "recording" => match value.as_str() {
            Some("full") => Ok(Action::Recording(Recording::Full)),
            Some("flows") => Ok(Action::Recording(Recording::Flows)),
            Some("off") => Ok(Action::Recording(Recording::Off)),
            _ => Err(ApiError::validation("recording is full, flows or off.")),
        },
        _ => Err(ApiError::validation(
            "what is allow_all, egress_mode, dns, internet, mcp or recording.",
        )),
    }
}

/// Close the open windows of one sandbox, optionally only those of one channel.
///
/// `Some("all")` closes only channel `all`; `None` closes every channel.
async fn close_windows(
    ctx: &ServiceContext,
    principal: &Principal,
    sandbox_id: Uuid,
    channel: Option<&str>,
) -> Result<(), ApiError> {
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let rows = policy_store::list_windows(&mut scope)
        .await
        .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    for row in rows {
        if row.sandbox_id != Some(sandbox_id) {
            continue;
        }
        if channel.is_some_and(|wanted| row.channel != wanted) {
            continue;
        }
        policy_service::close_window(ctx, principal, row.id).await?;
    }
    Ok(())
}

/// Move the recording mutes of a sandbox to one level.
///
/// `Off` mutes flows *and* bodies for up to a day, so the sandbox's traffic is
/// not recorded. What it does not do is hide a refusal: `telemetry_service`
/// still stores a denied network or DNS event as metadata, without its bodies,
/// so the alerts, the tree badge and the audit trail survive the mute (ADR 0066,
/// addendum). The permission is unchanged — a member may switch recording off,
/// because the user asked for the switch; the refusals are what stays visible.
async fn set_recording(
    ctx: &ServiceContext,
    principal: &Principal,
    sandbox_id: Uuid,
    level: Recording,
    minutes: Option<i32>,
) -> Result<(), ApiError> {
    let until = Utc::now() + Duration::minutes(minutes.map_or(DEFAULT_MUTE_MINUTES, i64::from));
    let (all, bodies) = match level {
        Recording::Full => (None, None),
        Recording::Flows => (None, Some(until)),
        Recording::Off => (Some(until), Some(until)),
    };
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    sandbox_store::set_telemetry_mute(&mut scope, sandbox_id, all, false)
        .await
        .map_err(ApiError::from)?;
    sandbox_store::set_telemetry_mute(&mut scope, sandbox_id, bodies, true)
        .await
        .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allow_all_needs_a_boolean() {
        assert!(parse("allow_all", &serde_json::json!(true)).is_ok());
        assert!(parse("allow_all", &serde_json::json!("yes")).is_err());
    }

    #[test]
    fn egress_mode_accepts_only_the_four_modes() {
        assert!(parse("egress_mode", &serde_json::json!("hybrid")).is_ok());
        assert!(parse("egress_mode", &serde_json::json!("direct")).is_err());
        assert!(parse("egress_mode", &serde_json::json!(true)).is_err());
    }

    #[test]
    fn recording_takes_three_words() {
        assert_eq!(
            parse("recording", &serde_json::json!("off")).ok(),
            Some(Action::Recording(Recording::Off))
        );
        assert!(parse("recording", &serde_json::json!("some")).is_err());
    }

    #[test]
    fn internet_maps_to_the_proxy_channel() {
        assert_eq!(
            parse("internet", &serde_json::json!(false)).ok(),
            Some(Action::Channel("proxy", false))
        );
    }

    #[test]
    fn an_unknown_control_is_refused() {
        assert!(parse("teleport", &serde_json::json!(true)).is_err());
    }
}
