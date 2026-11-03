//! Alerts: refused destinations awaiting an answer (package B08).

use chrono::{DateTime, Utc};
use uuid::Uuid;

use super::{require, ServiceContext};
use crate::domain::alerts::AlertView;
use crate::domain::auth::{Permission, Principal};
use crate::domain::telemetry::{Decision, LiveEvent, TelemetryEvent};
use crate::error::ApiError;
use crate::store::postgres::alerts as alert_store;
use crate::store::TenantScope;

/// Open alerts in the last `window_minutes`, optionally for one sandbox.
///
/// Row-level security on `sandboxes` already limits a member to their own sandboxes.
pub async fn list(
    ctx: &ServiceContext,
    principal: &Principal,
    window_minutes: i64,
    sandbox: Option<Uuid>,
) -> Result<Vec<AlertView>, ApiError> {
    require(principal, Permission::TelemetryRead)?;
    let since = Utc::now() - chrono::Duration::minutes(window_minutes);
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let items = alert_store::refused_grouped(&mut scope, since, sandbox)
        .await
        .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(items)
}

/// Publish an `alert` live event for every refused network or DNS event just stored.
///
/// Reads nothing from the database (the batch path is hot): the sandbox name and owner are not
/// known here, so the live alert carries the sandbox id as its name and no owner; the console
/// refreshes the list for the full view. `count` is 1 per event.
pub async fn publish_if_refused(
    ctx: &ServiceContext,
    environment_id: Uuid,
    events: &[TelemetryEvent],
) {
    for event in events {
        let Some(alert) = alert_of(event) else { continue };
        let value = match serde_json::to_value(&alert) {
            Ok(value) => value,
            Err(error) => {
                tracing::warn!(%error, "cannot serialise an alert");
                continue;
            }
        };
        ctx.bus.publish(LiveEvent::Alert {
            environment_id,
            sandbox_id: Some(alert.sandbox_id),
            alert: value,
        });
    }
}

/// The alert a refused network or DNS event stands for; `None` for anything else.
fn alert_of(event: &TelemetryEvent) -> Option<AlertView> {
    match event {
        TelemetryEvent::Network(e) if e.decision == Decision::Deny => {
            let sandbox_id = e.sandbox_id?;
            let port = e.port.unwrap_or(0);
            Some(view(
                sandbox_id,
                &e.destination,
                port,
                &e.protocol,
                e.channel.as_deref().unwrap_or("proxy"),
                e.ts,
            ))
        }
        TelemetryEvent::Dns(e) if e.decision == Decision::Deny => {
            let sandbox_id = e.sandbox_id?;
            Some(view(sandbox_id, &e.hostname, 0, "dns", "dns", e.ts))
        }
        _ => None,
    }
}

/// One single-event alert.
fn view(
    sandbox_id: Uuid,
    host: &str,
    port: i32,
    protocol: &str,
    channel: &str,
    ts: DateTime<Utc>,
) -> AlertView {
    AlertView {
        id: crate::domain::alerts::alert_id(sandbox_id, host, port),
        sandbox_id,
        sandbox_name: sandbox_id.to_string(),
        host: host.to_ascii_lowercase(),
        port,
        protocol: protocol.to_string(),
        channel: channel.to_string(),
        count: 1,
        first_at: ts,
        last_at: ts,
        decision: "blocked".to_string(),
        reason: "no_rule".to_string(),
        would_deny: false,
        owner: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::telemetry::DnsEvent;

    fn dns(decision: Decision) -> TelemetryEvent {
        TelemetryEvent::Dns(DnsEvent {
            id: Uuid::nil(),
            environment_id: Uuid::nil(),
            sandbox_id: Some(Uuid::nil()),
            ts: Utc::now(),
            hostname: "Bad.Example.net".to_string(),
            resolver: None,
            decision,
            binary: None,
            correlation_id: None,
        })
    }

    #[test]
    fn a_refused_lookup_becomes_an_alert() {
        let alert = alert_of(&dns(Decision::Deny));
        assert_eq!(alert.map(|a| (a.host, a.channel, a.count)), Some(("bad.example.net".into(), "dns".into(), 1)));
    }

    #[test]
    fn an_allowed_lookup_is_no_alert() {
        assert!(alert_of(&dns(Decision::Allow)).is_none());
    }
}
