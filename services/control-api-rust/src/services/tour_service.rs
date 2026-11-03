//! First-run tours: which tours the caller finished, and marking one as finished.

use super::ServiceContext;
use crate::domain::auth::Principal;
use crate::error::ApiError;
use crate::store::postgres::tours as tour_store;

/// The longest tour name accepted; names are short ids such as `shell-v1`.
const MAX_TOUR_NAME: usize = 64;

/// Check that a tour name is a short id of lower-case letters, digits, `-` and `_`.
fn validate_name(tour: &str) -> Result<(), ApiError> {
    let ok = !tour.is_empty()
        && tour.len() <= MAX_TOUR_NAME
        && tour
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_');
    if ok {
        Ok(())
    } else {
        Err(ApiError::validation("A tour name is 1-64 characters of a-z, 0-9, '-' and '_'."))
    }
}

/// Mark a tour as seen for the caller; own row only, any signed-in user, idempotent.
pub async fn mark_seen(ctx: &ServiceContext, principal: &Principal, tour: &str) -> Result<(), ApiError> {
    validate_name(tour)?;
    tour_store::mark_seen(&ctx.pool, principal.user_id, tour)
        .await
        .map_err(ApiError::from)
}

/// The tours the caller has finished; own rows only, so the console can skip them.
pub async fn seen(ctx: &ServiceContext, principal: &Principal) -> Result<Vec<String>, ApiError> {
    tour_store::seen(&ctx.pool, principal.user_id)
        .await
        .map_err(ApiError::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_tour_id_is_accepted() {
        assert!(validate_name("shell-v1").is_ok());
    }

    #[test]
    fn an_empty_or_odd_name_is_refused() {
        assert!(validate_name("").is_err());
        assert!(validate_name("Shell v1").is_err());
        assert!(validate_name(&"a".repeat(65)).is_err());
    }
}
