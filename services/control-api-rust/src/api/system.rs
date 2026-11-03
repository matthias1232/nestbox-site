//! System endpoints: health, readiness, runtime facts, and the API document.
//!
//! Health and readiness answer different questions on purpose:
//! `healthz` says the process is alive, `readyz` says it can actually serve —
//! which means the database answered. A load balancer that only checks liveness
//! will send traffic to a process that cannot reach its store.
//!
//! `readyz` also reports how much disk is left. A full appliance is not a
//! readiness failure — the database still answers — but it is the failure that
//! took the console down once, so the answer carries the number and the console
//! can warn before writes start failing.

use std::time::Duration;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde::Serialize;

use crate::api::principal::Auth;
use crate::error::ApiError;
use crate::openapi;
use crate::state::AppState;

/// Liveness: the process is running.
pub async fn healthz() -> &'static str {
    "ok"
}

/// How long readiness waits for the host load before reporting no disk figure.
///
/// A probe must not hang; two seconds is several times what a local runner
/// needs and short enough that a dead runner only delays an answer, it does not
/// stall the load balancer.
const HOST_LOAD_TIMEOUT: Duration = Duration::from_secs(2);

/// Free bytes on the host's disk, taken from a host-load answer.
///
/// The runner reports `disk` as `{"total": …, "used": …, "free": …}`, or `null`
/// when it cannot read the mount. The subtraction saturates: a container
/// filesystem that states `used` above `total` yields 0, never a negative
/// number that a caller could read as "some space is left".
fn free_disk_bytes(load: &serde_json::Value) -> Option<i64> {
    let disk = load.get("disk")?;
    let total = disk.get("total")?.as_i64()?;
    let used = disk.get("used")?.as_i64()?;
    // `saturating_sub` covers the arithmetic overflow, `max(0)` the case the
    // container reports more used than total; neither can answer a negative.
    Some(total.saturating_sub(used).max(0))
}

/// Readiness: the process can serve, including its database.
///
/// The status code is unchanged by the disk figure: a full disk is reported,
/// not enforced here. Failing readiness would take the console down, which is
/// the state this number exists to warn about.
pub async fn readyz(State(state): State<AppState>) -> impl IntoResponse {
    let readiness = if crate::store::postgres::is_ready(state.pool()).await {
        (StatusCode::OK, "ready")
    } else {
        tracing::error!("readiness check failed");
        (StatusCode::SERVICE_UNAVAILABLE, "database unavailable")
    };
    let (status, reason) = readiness;
    let probe = tokio::time::timeout(HOST_LOAD_TIMEOUT, state.services.runtime.host_load());
    let disk_free_bytes = match probe.await {
        Ok(Ok(load)) => free_disk_bytes(&load),
        _ => None,
    };

    (
        status,
        Json(serde_json::json!({
            "status": reason,
            "disk_free_bytes": disk_free_bytes,
        })),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn free_disk_bytes_subtracts_and_saturates() {
        let load = serde_json::json!({
            "disk": { "total": 116_000_000_000_i64, "used": 16_000_000_000_i64, "free": 100_000_000_000_i64 }
        });
        assert_eq!(free_disk_bytes(&load), Some(100_000_000_000));

        let over_reported = serde_json::json!({ "disk": { "total": 10, "used": 20 } });
        assert_eq!(free_disk_bytes(&over_reported), Some(0));
    }

    #[test]
    fn free_disk_bytes_is_none_without_a_disk_figure() {
        assert_eq!(free_disk_bytes(&serde_json::json!({})), None);
        assert_eq!(free_disk_bytes(&serde_json::json!({ "disk": null })), None);
        assert_eq!(free_disk_bytes(&serde_json::json!({ "disk": { "total": 10 } })), None);
    }
}

#[derive(Serialize)]
pub struct RuntimeInfo {
    pub runtime: String,
    pub network: String,
    pub api_version: &'static str,
}

/// What is actually executing sandboxes, stated honestly.
pub async fn runtime_info(State(state): State<AppState>) -> Json<RuntimeInfo> {
    Json(RuntimeInfo {
        runtime: state.services.runtime.name().to_string(),
        network: state.services.runtime.network().to_string(),
        api_version: "v1",
    })
}

/// The OpenAPI document.
pub async fn openapi() -> Json<serde_json::Value> {
    Json(openapi::document())
}

/// A minimal documentation page.
///
/// Swagger UI is deliberately not embedded: it is a large third-party bundle
/// served from the same origin as an authenticated console, and the generated
/// document plus `curl` is enough for the operator this ships to. The page says
/// so rather than pretending a full UI exists.
pub async fn docs() -> impl IntoResponse {
    (
        StatusCode::OK,
        [("content-type", "text/html; charset=utf-8")],
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8">
<title>Nestbox API</title>
<style>body{font:16px/1.6 system-ui,sans-serif;max-width:52rem;margin:3rem auto;padding:0 1rem}
code{background:#eee;padding:.1rem .3rem;border-radius:3px}</style></head><body>
<h1>Nestbox API v1</h1>
<p>The machine-readable contract is at <a href="/api/v1/openapi.json">/api/v1/openapi.json</a>
and also at <code>/api/schema/v1</code>. The prefix <code>/api/v1/</code> is frozen;
a breaking change gets a new prefix.</p>
<p>Authenticate with a session cookie from <code>POST /api/v1/auth/login</code>
(mutating requests also need the <code>x-csrf-token</code> header), or with
<code>Authorization: Token &lt;key&gt;</code> for automation.</p>
</body></html>"#,
    )
}

/// The environments this user may access.
pub async fn environments(
    State(state): State<AppState>,
    auth: Auth,
) -> Result<Json<serde_json::Value>, ApiError> {
    let environments =
        crate::services::environment_service::environments_for_user(&state.services, &auth.principal)
            .await?;
    Ok(Json(serde_json::json!({ "items": environments })))
}

/// A summary of the environment the request is scoped to.
pub async fn current_environment(
    State(state): State<AppState>,
    auth: Auth,
) -> Result<Json<serde_json::Value>, ApiError> {
    let summary =
        crate::services::environment_service::summary(&state.services, &auth.principal).await?;

    Ok(Json(serde_json::json!({
        "id": summary.id,
        "name": summary.name,
        "slug": summary.slug,
        "sandboxes": summary.sandbox_count,
        "gateways": summary.gateway_count,
        "running": summary.running_count,
        "role": auth.principal.role.as_str(),
        "display_name": auth.principal.display_name,
    })))
}

/// Recent audit events.
pub async fn audit_events(
    State(state): State<AppState>,
    auth: Auth,
) -> Result<Json<serde_json::Value>, ApiError> {
    let events =
        crate::services::audit_service::list(&state.services, &auth.principal, 200).await?;
    Ok(Json(serde_json::json!({ "items": events })))
}
