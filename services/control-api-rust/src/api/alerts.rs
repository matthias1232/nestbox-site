//! Alerts: `GET /alerts` (package B08).

use axum::Json;
use axum::extract::{Query, State};
use serde::Deserialize;
use uuid::Uuid;

use crate::api::principal::Auth;
use crate::error::ApiError;
use crate::services::alert_service;
use crate::state::AppState;

/// The query of the alert list.
#[derive(Debug, Deserialize)]
pub struct AlertQuery {
    /// A window such as `15m`, `1h` or `24h`; default `15m`.
    #[serde(default)]
    pub window: Option<String>,
    #[serde(default)]
    pub sandbox: Option<Uuid>,
}

/// Parse `15m`, `2h` or `1d` into minutes; anything else is a validation error.
fn window_minutes(value: Option<&str>) -> Result<i64, ApiError> {
    let Some(value) = value.filter(|v| !v.is_empty()) else { return Ok(15) };
    let (digits, unit) = value.split_at(value.len().saturating_sub(1));
    let amount: i64 = digits
        .parse()
        .map_err(|_| ApiError::validation("window must look like 15m, 2h or 1d."))?;
    let minutes = match unit {
        "m" => amount,
        "h" => amount * 60,
        "d" => amount * 1440,
        _ => return Err(ApiError::validation("window must look like 15m, 2h or 1d.")),
    };
    Ok(minutes.clamp(1, 7 * 1440))
}

/// Refused destinations nobody answered yet.
pub async fn list(
    State(state): State<AppState>,
    auth: Auth,
    Query(query): Query<AlertQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let minutes = window_minutes(query.window.as_deref())?;
    let items = alert_service::list(&state.services, &auth.principal, minutes, query.sandbox).await?;
    Ok(Json(serde_json::json!({ "items": items })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_parse_to_minutes() {
        assert_eq!(window_minutes(None).ok(), Some(15));
        assert_eq!(window_minutes(Some("30m")).ok(), Some(30));
        assert_eq!(window_minutes(Some("2h")).ok(), Some(120));
        assert!(window_minutes(Some("soon")).is_err());
    }
}
