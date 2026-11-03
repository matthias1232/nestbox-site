//! Permissions per scope (package B12 builds it).

use uuid::Uuid;

use super::ServiceContext;
use crate::domain::auth::Principal;
use crate::domain::permissions::ScopeType;
use crate::error::ApiError;

/// Who has which role in a scope, plus open invitations (contract C13).
pub async fn list(
    _ctx: &ServiceContext,
    _principal: &Principal,
    _scope_type: ScopeType,
    _scope_id: Uuid,
) -> Result<serde_json::Value, ApiError> {
    Err(ApiError::internal("not built: B12"))
}

/// Give a user a role in a scope.
pub async fn set(
    _ctx: &ServiceContext,
    _principal: &Principal,
    _user_id: Uuid,
    _scope_type: ScopeType,
    _scope_id: Uuid,
    _role: &str,
) -> Result<(), ApiError> {
    Err(ApiError::internal("not built: B12"))
}

/// Take a user's role in a scope away.
pub async fn remove(
    _ctx: &ServiceContext,
    _principal: &Principal,
    _user_id: Uuid,
    _scope_type: ScopeType,
    _scope_id: Uuid,
) -> Result<(), ApiError> {
    Err(ApiError::internal("not built: B12"))
}
