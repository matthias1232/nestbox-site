//! First-run tours (package F18 builds the rest).

use super::ServiceContext;
use crate::domain::auth::Principal;
use crate::error::ApiError;

/// Mark a tour as seen for the caller; own row only.
pub async fn mark_seen(_ctx: &ServiceContext, _principal: &Principal, _tour: &str) -> Result<(), ApiError> {
    Err(ApiError::internal("not built: F18"))
}
