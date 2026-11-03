//! Refused destinations grouped into alerts (package B08 fills the query).

use uuid::Uuid;

use super::super::TenantScope;
use crate::domain::alerts::AlertView;
use crate::error::StoreError;

/// Refused destinations in the last `window_minutes`, grouped per sandbox, host and port.
pub async fn refused_grouped(
    _scope: &mut TenantScope<'_>,
    _window_minutes: i64,
    _sandbox: Option<Uuid>,
) -> Result<Vec<AlertView>, StoreError> {
    Ok(Vec::new())
}
