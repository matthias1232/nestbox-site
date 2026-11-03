//! Global search over everything the caller may see (package B02 builds it).

use super::ServiceContext;
use crate::domain::auth::Principal;
use crate::domain::search::{SearchHit, SearchParams};
use crate::error::ApiError;

/// Search the kinds named in `params`, best hits first.
pub async fn search(
    _ctx: &ServiceContext,
    _principal: &Principal,
    _params: &SearchParams,
) -> Result<Vec<SearchHit>, ApiError> {
    Err(ApiError::internal("not built: B02"))
}
