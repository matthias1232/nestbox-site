//! Effective mounts of a sandbox (package B09 builds it).

use uuid::Uuid;

use super::ServiceContext;
use crate::domain::auth::Principal;
use crate::domain::mounts::{EffectiveMounts, MountSpec};
use crate::error::ApiError;

/// What the sandbox has mounted, what it could mount, and whether a recreate is pending.
pub async fn effective(
    _ctx: &ServiceContext,
    _principal: &Principal,
    _sandbox_id: Uuid,
) -> Result<EffectiveMounts, ApiError> {
    Err(ApiError::internal("not built: B09"))
}

/// Make the mount set equal `roots`; with `apply == "now"` recreate the sandbox.
pub async fn set(
    _ctx: &ServiceContext,
    _principal: &Principal,
    _sandbox_id: Uuid,
    _roots: Vec<MountSpec>,
    _apply: &str,
) -> Result<EffectiveMounts, ApiError> {
    Err(ApiError::internal("not built: B09"))
}
