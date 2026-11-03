//! The inventory tree the caller may see (package B01 builds it).

use super::ServiceContext;
use crate::domain::auth::Principal;
use crate::domain::inventory::{TreeNode, TreeView};
use crate::error::ApiError;

/// The tree for one view, restricted to what the caller may see.
pub async fn tree(
    _ctx: &ServiceContext,
    _principal: &Principal,
    _view: TreeView,
) -> Result<TreeNode, ApiError> {
    Err(ApiError::internal("not built: B01"))
}
