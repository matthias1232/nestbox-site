//! Global search: `GET /search` (package B02).

use axum::Json;
use axum::extract::{Query, State};
use serde::Deserialize;
use uuid::Uuid;

use crate::api::principal::Auth;
use crate::domain::search::{SearchKind, SearchParams};
use crate::error::ApiError;
use crate::services::search_service;
use crate::state::AppState;

/// The query string of a search.
#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    #[serde(default)]
    pub q: String,
    /// Comma-separated kinds.
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub environment: Option<Uuid>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub limit: Option<i64>,
}

/// Search sandboxes, nodes, models and more.
pub async fn search(
    State(state): State<AppState>,
    auth: Auth,
    Query(query): Query<SearchQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let mut kinds = Vec::new();
    for part in query.kind.as_deref().unwrap_or("").split(',').filter(|p| !p.trim().is_empty()) {
        kinds.push(
            SearchKind::parse(part.trim())
                .ok_or_else(|| ApiError::validation(format!("Unknown search kind '{part}'.")))?,
        );
    }
    let params = SearchParams {
        q: query.q,
        kinds,
        environment: query.environment,
        status: query.status,
        limit: query.limit.unwrap_or(20).clamp(1, 50),
    };
    let items = search_service::search(&state.services, &auth.principal, &params).await?;
    Ok(Json(serde_json::json!({ "items": items })))
}
