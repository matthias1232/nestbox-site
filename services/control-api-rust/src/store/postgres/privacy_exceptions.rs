//! Rows of `privacy_exceptions`: insert, read, delete.
//!
//! Matching is a service concern; this file only moves rows. Every statement
//! binds `environment_id = $1` from the [`TenantScope`], so the application's
//! filter and the database's row-level security cannot disagree.
//!
//! No statement here writes a value. The table holds a salted fingerprint at
//! most, so a caller cannot turn an exception into a copy of the content even
//! by mistake.

use chrono::{DateTime, Utc};
use sqlx::Row;
use uuid::Uuid;

use super::super::TenantScope;
use crate::domain::privacy::{Action, Detector};
use crate::domain::privacy_exception::PrivacyException;
use crate::error::StoreError;

/// The exception columns, named so a schema change is a mapping error rather
/// than a silently swapped field.
const COLUMNS: &str = "id, environment_id, sandbox_id, detector, value_fingerprint, \
     destination, action, expires_at, reason, created_by, created_at";

/// The same columns, qualified for the query that joins `sandboxes`.
const COLUMNS_QUALIFIED: &str = "e.id, e.environment_id, e.sandbox_id, e.detector, \
     e.value_fingerprint, e.destination, e.action, e.expires_at, e.reason, e.created_by, \
     e.created_at";

/// What an incident contributes to an exception made from it.
///
/// The value is never part of this: `value_fingerprint` is the salted hash the
/// incident already holds, and `owner_user_id` is who may grant the exception
/// when the environment lets owners decide.
pub struct IncidentBasis {
    pub sandbox_id: Uuid,
    pub detector: Detector,
    /// `None` when the finding was recorded without a hash; a value exception
    /// then cannot be made, and the caller reports that as a validation error.
    pub value_fingerprint: Option<String>,
    pub destination: Option<String>,
    /// The sandbox's owner, for the owner-may-grant check.
    pub owner_user_id: Option<Uuid>,
}

/// Write one exception.
///
/// The row is bound explicitly, including the id, so the caller's object and
/// the stored row are the same thing and the audit detail cannot describe a
/// different row than the one written.
pub async fn insert(
    scope: &mut TenantScope<'_>,
    exception: &PrivacyException,
) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO privacy_exceptions \
            (id, environment_id, sandbox_id, detector, value_fingerprint, destination, \
             action, expires_at, reason, created_by, created_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
    )
    .bind(exception.id)
    .bind(scope.environment_id())
    .bind(exception.sandbox_id)
    .bind(exception.detector.as_str())
    .bind(exception.value_fingerprint.as_deref())
    .bind(exception.destination.as_deref())
    .bind(exception.action.as_str())
    .bind(exception.expires_at)
    .bind(&exception.reason)
    .bind(exception.created_by)
    .bind(exception.created_at)
    .execute(scope.connection())
    .await?;
    Ok(())
}

/// Exceptions that could apply to one sandbox now: its own and the
/// environment's.
///
/// Expired rows are filtered here, not only in the matcher, so a caller that
/// forgets the matcher still cannot honour an expired decision.
pub async fn active(
    scope: &mut TenantScope<'_>,
    sandbox_id: Uuid,
) -> Result<Vec<PrivacyException>, StoreError> {
    let statement = format!(
        "SELECT {COLUMNS} FROM privacy_exceptions \
          WHERE environment_id = $1 \
            AND (sandbox_id IS NULL OR sandbox_id = $2) \
            AND (expires_at IS NULL OR expires_at > now()) \
          ORDER BY created_at DESC LIMIT 500"
    );
    let rows = sqlx::query(&statement)
        .bind(scope.environment_id())
        .bind(sandbox_id)
        .fetch_all(scope.connection())
        .await?;
    rows.iter().map(exception_from_row).collect()
}

/// The exceptions a principal may see.
///
/// `owner` is the caller's user id for a non-admin, and `None` for an admin:
/// the same query then returns either only the rows of that person's sandboxes
/// or everything in the environment. The join is `LEFT` because an environment
/// exception has no sandbox row.
pub async fn list(
    scope: &mut TenantScope<'_>,
    sandbox_id: Option<Uuid>,
    owner: Option<Uuid>,
) -> Result<Vec<PrivacyException>, StoreError> {
    let statement = format!(
        "SELECT {COLUMNS_QUALIFIED} FROM privacy_exceptions e \
           LEFT JOIN sandboxes s ON s.id = e.sandbox_id \
                                 AND s.environment_id = e.environment_id \
          WHERE e.environment_id = $1 \
            AND ($2::uuid IS NULL OR e.sandbox_id = $2) \
            AND ($3::uuid IS NULL OR s.owner_user_id = $3) \
          ORDER BY e.created_at DESC LIMIT 500"
    );
    let rows = sqlx::query(&statement)
        .bind(scope.environment_id())
        .bind(sandbox_id)
        .bind(owner)
        .fetch_all(scope.connection())
        .await?;
    rows.iter().map(exception_from_row).collect()
}

/// One exception by id, or `None` when it does not exist in this environment.
pub async fn get(
    scope: &mut TenantScope<'_>,
    id: Uuid,
) -> Result<Option<PrivacyException>, StoreError> {
    let statement = format!(
        "SELECT {COLUMNS} FROM privacy_exceptions WHERE environment_id = $1 AND id = $2"
    );
    let row = sqlx::query(&statement)
        .bind(scope.environment_id())
        .bind(id)
        .fetch_optional(scope.connection())
        .await?;
    row.as_ref().map(exception_from_row).transpose()
}

/// Remove one exception; `false` when there was nothing to remove.
pub async fn delete(scope: &mut TenantScope<'_>, id: Uuid) -> Result<bool, StoreError> {
    let result = sqlx::query("DELETE FROM privacy_exceptions WHERE environment_id = $1 AND id = $2")
        .bind(scope.environment_id())
        .bind(id)
        .execute(scope.connection())
        .await?;
    Ok(result.rows_affected() > 0)
}

/// The incident an exception is made from, with its sandbox's owner.
///
/// `None` is a 404 at the service: the incident must exist in this environment,
/// and that includes a member trying an incident of somebody else's sandbox
/// (row-level security already hides it).
pub async fn incident_for_exception(
    scope: &mut TenantScope<'_>,
    incident_id: Uuid,
) -> Result<Option<IncidentBasis>, StoreError> {
    let row = sqlx::query(
        "SELECT i.sandbox_id, i.detector, i.value_fingerprint, i.destination, \
                s.owner_user_id \
           FROM privacy_incidents i \
           JOIN sandboxes s ON s.id = i.sandbox_id \
                            AND s.environment_id = i.environment_id \
          WHERE i.environment_id = $1 AND i.id = $2",
    )
    .bind(scope.environment_id())
    .bind(incident_id)
    .fetch_optional(scope.connection())
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let detector: String = row.try_get("detector")?;
    let detector = Detector::parse(&detector)
        .ok_or_else(|| StoreError::UnexpectedData(format!("unknown detector: {detector}")))?;
    Ok(Some(IncidentBasis {
        sandbox_id: row.try_get("sandbox_id")?,
        detector,
        value_fingerprint: row.try_get("value_fingerprint")?,
        destination: row.try_get("destination")?,
        owner_user_id: row.try_get("owner_user_id")?,
    }))
}

/// One exception row, mapped to the domain type.
///
/// A spelling the code does not know is an error rather than a skip: a row the
/// matcher silently dropped would say "allowed" in the list and "blocked" in
/// the filter.
fn exception_from_row(row: &sqlx::postgres::PgRow) -> Result<PrivacyException, StoreError> {
    let detector: String = row.try_get("detector")?;
    let detector = Detector::parse(&detector)
        .ok_or_else(|| StoreError::UnexpectedData(format!("unknown detector: {detector}")))?;
    let action: String = row.try_get("action")?;
    let action = Action::parse(&action)
        .ok_or_else(|| StoreError::UnexpectedData(format!("unknown exception action: {action}")))?;
    Ok(PrivacyException {
        id: row.try_get("id")?,
        environment_id: row.try_get("environment_id")?,
        sandbox_id: row.try_get("sandbox_id")?,
        detector,
        value_fingerprint: row.try_get("value_fingerprint")?,
        destination: row.try_get("destination")?,
        action,
        expires_at: row.try_get::<Option<DateTime<Utc>>, _>("expires_at")?,
        reason: row.try_get("reason")?,
        created_by: row.try_get("created_by")?,
        created_at: row.try_get("created_at")?,
    })
}
