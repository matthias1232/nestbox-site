//! Search queries, one per kind (package B02 fills them).

use super::super::TenantScope;
use crate::domain::search::SearchHit;
use crate::error::StoreError;

/// Sandboxes whose name or id matches `q`.
pub async fn sandboxes(_scope: &mut TenantScope<'_>, _q: &str, _limit: i64) -> Result<Vec<SearchHit>, StoreError> {
    Ok(Vec::new())
}

/// Nodes whose name or id matches `q`.
pub async fn nodes(_scope: &mut TenantScope<'_>, _q: &str, _limit: i64) -> Result<Vec<SearchHit>, StoreError> {
    Ok(Vec::new())
}

/// Catalogue models whose key or name matches `q`.
pub async fn models(_scope: &mut TenantScope<'_>, _q: &str, _limit: i64) -> Result<Vec<SearchHit>, StoreError> {
    Ok(Vec::new())
}

/// Policies whose name matches `q`.
pub async fn policies(_scope: &mut TenantScope<'_>, _q: &str, _limit: i64) -> Result<Vec<SearchHit>, StoreError> {
    Ok(Vec::new())
}

/// Users whose name or email matches `q`.
pub async fn users(_scope: &mut TenantScope<'_>, _q: &str, _limit: i64) -> Result<Vec<SearchHit>, StoreError> {
    Ok(Vec::new())
}

/// Sites whose name matches `q`.
pub async fn sites(_scope: &mut TenantScope<'_>, _q: &str, _limit: i64) -> Result<Vec<SearchHit>, StoreError> {
    Ok(Vec::new())
}

/// Model clusters whose name matches `q`.
pub async fn clusters(_scope: &mut TenantScope<'_>, _q: &str, _limit: i64) -> Result<Vec<SearchHit>, StoreError> {
    Ok(Vec::new())
}

/// Jobs whose kind or id matches `q`.
pub async fn jobs(_scope: &mut TenantScope<'_>, _q: &str, _limit: i64) -> Result<Vec<SearchHit>, StoreError> {
    Ok(Vec::new())
}
