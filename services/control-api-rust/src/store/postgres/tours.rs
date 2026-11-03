//! Which guided tours a user has finished. The table is per user, not per data center.

use uuid::Uuid;

use crate::error::StoreError;

/// Remember that the user finished a tour. Idempotent.
pub async fn mark_seen(pool: &sqlx::PgPool, user_id: Uuid, tour: &str) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO ui_tours_seen (user_id, tour) VALUES ($1, $2)
         ON CONFLICT (user_id, tour) DO NOTHING",
    )
    .bind(user_id)
    .bind(tour)
    .execute(pool)
    .await
    .map_err(StoreError::Database)?;
    Ok(())
}

/// The tours the user has finished.
pub async fn seen(pool: &sqlx::PgPool, user_id: Uuid) -> Result<Vec<String>, StoreError> {
    sqlx::query_scalar::<_, String>("SELECT tour FROM ui_tours_seen WHERE user_id = $1 ORDER BY tour")
        .bind(user_id)
        .fetch_all(pool)
        .await
        .map_err(StoreError::Database)
}
