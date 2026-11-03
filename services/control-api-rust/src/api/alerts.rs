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
    /// One of `5m`, `15m`, `1h`, `24h`; default `15m`.
    #[serde(default)]
    pub window: Option<String>,
    #[serde(default)]
    pub sandbox: Option<Uuid>,
}

/// Parse a window word into minutes; anything but `5m|15m|1h|24h` is a validation error.
fn window_minutes(value: Option<&str>) -> Result<i64, ApiError> {
    let word = value.filter(|v| !v.is_empty()).unwrap_or("15m");
    crate::domain::alerts::window(word)
        .map(|duration| duration.num_minutes())
        .ok_or_else(|| ApiError::validation("window must be one of 5m, 15m, 1h or 24h."))
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
        assert_eq!(window_minutes(Some("5m")).ok(), Some(5));
        assert_eq!(window_minutes(Some("1h")).ok(), Some(60));
        assert!(window_minutes(Some("soon")).is_err());
    }
}
