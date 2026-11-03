//! Effective mounts of a sandbox (package B09).

use axum::Json;
use axum::extract::{Path, State};
use serde::Deserialize;
use uuid::Uuid;

use crate::api::principal::Auth;
use crate::domain::mounts::{EffectiveMounts, MountSpec};
use crate::error::ApiError;
use crate::services::mount_service;
use crate::state::AppState;

/// What the sandbox has mounted and could mount.
pub async fn get(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
) -> Result<Json<EffectiveMounts>, ApiError> {
    Ok(Json(mount_service::effective(&state.services, &auth.principal, id).await?))
}

/// The body of a mount change.
#[derive(Debug, Deserialize)]
pub struct PutMounts {
    pub roots: Vec<MountSpec>,
    /// `now` or `later`.
    #[serde(default)]
    pub apply: Option<String>,
}

/// Make the mount set equal the body.
pub async fn put(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
    Json(body): Json<PutMounts>,
) -> Result<Json<EffectiveMounts>, ApiError> {
    let apply = body.apply.as_deref().unwrap_or("later");
    if apply != "now" && apply != "later" {
        return Err(ApiError::validation("apply must be now or later."));
    }
    Ok(Json(mount_service::set(&state.services, &auth.principal, id, body.roots, apply).await?))
}
