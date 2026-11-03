//! Privacy settings and incidents (`0056_privacy_layer.sql`).
//!
//! Two aggregates, one file because they are one subject: the policy row a
//! sandbox inherits and the evidence rows of what that policy did. Both are
//! tenant-scoped and both are read through [`TenantScope`], so the database's
//! row-level security and the `environment_id` bind agree on every statement.
//!
//! Nothing here ever writes a matched value. `insert_incident` takes a masked
//! excerpt and a SHA-256 fingerprint, and the schema has no column that could
//! hold anything else — the rule survives a caller that forgets it.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use sqlx::Row;
use uuid::Uuid;

use super::super::TenantScope;
use crate::domain::privacy::{
    Action, Detector, IncidentAction, IncidentSource, PrivacyIncident, PrivacySettings, PrivacyMode,
};
use crate::error::StoreError;

/// The settings columns, named so a schema change is a mapping error rather than
/// a silently swapped field.
const SETTINGS_COLUMNS: &str = "sandbox_id, mode, detectors, actions, \
     block_cloud_egress, retention_days, enforce_hosts, owners_may_grant_exceptions, \
     admin_chat_view, updated_by, updated_at";

/// The incident columns, named for the same reason.
const INCIDENT_COLUMNS: &str = "id, environment_id, sandbox_id, detector, action, severity, \
     masked_value, value_fingerprint, destination, cloud_delivery, direction, \
     requested_action, mode, replacement, source, method, path, model, mcp_tool, \
     conversation_id, occurrences, last_seen_at, created_at, request_ref, exception_id";

/// One incident to write: a masked excerpt, a fingerprint, and what happened.
///
/// No field carries the value, which is the point: the type cannot express a
/// write of the content even if a caller wanted one.
pub struct NewIncident<'a> {
    /// The sandbox the content belongs to; the incident row requires one.
    pub sandbox_id: Uuid,
    /// What fired.
    pub detector: Detector,
    /// What the policy did.
    pub action: IncidentAction,
    /// `info`, `warning` or `critical`; derived from the action by the service.
    pub severity: &'a str,
    /// First and last characters of the value, stars between.
    ///
    /// `None` for a row that has no value behind it: a refusal the proxy
    /// reported (check unavailable, body too large) is evidence about a request,
    /// not about content.
    pub masked_value: Option<&'a str>,
    /// Lowercase SHA-256 of the value; `None` where there is no value.
    pub value_fingerprint: Option<&'a str>,
    /// Where the content was going, when the caller named a destination.
    pub destination: Option<&'a str>,
    /// Whether the content counted as a delivery to a cloud destination.
    pub cloud_delivery: bool,
    /// `inbound` or `outbound`.
    pub direction: &'a str,
    /// What the policy asked for; differs from `action` in `observe`.
    pub requested_action: IncidentAction,
    /// The mode in force when the finding happened.
    pub mode: &'a str,
    /// The substitute the detector wrote, never the value.
    pub replacement: Option<&'a str>,
    /// Which hop saw the content.
    pub source: IncidentSource,
    /// The request the finding belonged to, when known.
    pub method: Option<&'a str>,
    pub path: Option<&'a str>,
    pub model: Option<&'a str>,
    pub mcp_tool: Option<&'a str>,
    pub conversation_id: Option<&'a str>,
    /// The proxy's `x-request-id`, when it sent one.
    pub request_ref: Option<&'a str>,
    /// The exception that allowed the finding, when one did.
    pub exception_id: Option<Uuid>,
}

/// Whether a sandbox exists in this environment.
///
/// It lives here rather than in `sandboxes.rs` for the same reason the ingest
/// node lookup lives in `telemetry.rs`: it exists only for this write path,
/// which must know that the sandbox it is about to write an incident for is real
/// before it writes one. Reading the whole sandbox row (its joins, its spec)
/// would make a per-message inspection pay for a screen it never shows.
pub async fn sandbox_exists(
    scope: &mut TenantScope<'_>,
    sandbox_id: Uuid,
) -> Result<bool, StoreError> {
    let found: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM sandboxes WHERE environment_id = $1 AND id = $2",
    )
    .bind(scope.environment_id())
    .bind(sandbox_id)
    .fetch_optional(scope.connection())
    .await?;
    Ok(found.is_some())
}

/// The settings row for one scope, or `None` when nothing was saved.
///
/// `sandbox_id: None` reads the environment default row. The comparison is
/// `IS NOT DISTINCT FROM` because the default row's key *is* NULL, and `= NULL`
/// matches nothing — the classic reason a "default" configuration silently never
/// applies.
pub async fn settings_for(
    scope: &mut TenantScope<'_>,
    sandbox_id: Option<Uuid>,
) -> Result<Option<PrivacySettings>, StoreError> {
    let statement = format!(
        "SELECT {SETTINGS_COLUMNS} FROM privacy_settings \
          WHERE environment_id = $1 AND sandbox_id IS NOT DISTINCT FROM $2"
    );
    let row = sqlx::query(&statement)
        .bind(scope.environment_id())
        .bind(sandbox_id)
        .fetch_optional(scope.connection())
        .await?;
    let Some(row) = row else {
        return Ok(None);
    };

    let mode: String = row.try_get("mode")?;
    let mode = PrivacyMode::parse(&mode)
        .ok_or_else(|| StoreError::UnexpectedData(format!("unknown privacy mode: {mode}")))?;

    let names: Vec<String> = row.try_get("detectors")?;
    let mut detectors = Vec::with_capacity(names.len());
    for name in names {
        let detector = Detector::parse(&name)
            .ok_or_else(|| StoreError::UnexpectedData(format!("unknown detector: {name}")))?;
        detectors.push(detector);
    }

    let actions: serde_json::Value = row.try_get("actions")?;
    let actions: BTreeMap<Detector, Action> = serde_json::from_value(actions)
        .map_err(|error| StoreError::UnexpectedData(format!("privacy actions: {error}")))?;

    let updated_at: DateTime<Utc> = row.try_get("updated_at")?;

    Ok(Some(PrivacySettings {
        sandbox_id: row.try_get("sandbox_id")?,
        mode,
        detectors,
        actions,
        block_cloud_egress: row.try_get("block_cloud_egress")?,
        retention_days: row.try_get("retention_days")?,
        enforce_hosts: row.try_get("enforce_hosts")?,
        owners_may_grant_exceptions: row.try_get("owners_may_grant_exceptions")?,
        admin_chat_view: row.try_get("admin_chat_view")?,
        updated_by: row.try_get("updated_by")?,
        updated_at: Some(updated_at),
    }))
}

/// Write the settings row for its scope, replacing what was there.
///
/// The conflict target is the named constraint because the default row's key
/// contains a NULL, and PostgreSQL's inference for a nullable key is only
/// reliable through the constraint name. Two concurrent saves therefore collide
/// on the constraint instead of producing two "defaults".
pub async fn save_settings(
    scope: &mut TenantScope<'_>,
    settings: &PrivacySettings,
) -> Result<(), StoreError> {
    let detectors: Vec<String> = settings
        .detectors
        .iter()
        .map(|detector| detector.as_str().to_string())
        .collect();
    let actions = serde_json::to_value(&settings.actions)
        .map_err(|error| StoreError::UnexpectedData(format!("privacy actions: {error}")))?;

    sqlx::query(
        "INSERT INTO privacy_settings (environment_id, sandbox_id, mode, detectors, actions, \
            block_cloud_egress, retention_days, enforce_hosts, owners_may_grant_exceptions, \
            admin_chat_view, updated_by, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, now()) \
         ON CONFLICT ON CONSTRAINT privacy_settings_scope DO UPDATE SET \
            mode = EXCLUDED.mode, \
            detectors = EXCLUDED.detectors, \
            actions = EXCLUDED.actions, \
            block_cloud_egress = EXCLUDED.block_cloud_egress, \
            retention_days = EXCLUDED.retention_days, \
            enforce_hosts = EXCLUDED.enforce_hosts, \
            owners_may_grant_exceptions = EXCLUDED.owners_may_grant_exceptions, \
            admin_chat_view = EXCLUDED.admin_chat_view, \
            updated_by = EXCLUDED.updated_by, \
            updated_at = now()",
    )
    .bind(scope.environment_id())
    .bind(settings.sandbox_id)
    .bind(settings.mode.as_str())
    .bind(&detectors)
    .bind(&actions)
    .bind(settings.block_cloud_egress)
    .bind(settings.retention_days)
    .bind(&settings.enforce_hosts)
    .bind(settings.owners_may_grant_exceptions)
    .bind(settings.admin_chat_view)
    .bind(settings.updated_by)
    .execute(scope.connection())
    .await?;
    Ok(())
}

/// Remove a sandbox's override so it inherits the environment default again.
///
/// Returns whether a row was removed: "inherit again" on a sandbox that had no
/// override is not an error, but the caller can tell the two apart for the audit
/// trail.
pub async fn delete_override(
    scope: &mut TenantScope<'_>,
    sandbox_id: Uuid,
) -> Result<bool, StoreError> {
    let result = sqlx::query(
        "DELETE FROM privacy_settings WHERE environment_id = $1 AND sandbox_id = $2",
    )
    .bind(scope.environment_id())
    .bind(sandbox_id)
    .execute(scope.connection())
    .await?;
    Ok(result.rows_affected() > 0)
}

/// Write one incident and return its id.
///
/// The row is evidence, so it is insert-only: there is no update or delete
/// statement for an incident except the retention sweep, which removes whole
/// days rather than rewriting one record.
pub async fn insert_incident(
    scope: &mut TenantScope<'_>,
    incident: NewIncident<'_>,
) -> Result<Uuid, StoreError> {
    // The same finding again within a day is the same secret seen again, not a
    // second secret. An agent resends its whole history on every turn, so a
    // fresh row per request turned one IBAN into hundreds of rows.
    let repeated: Option<Uuid> = sqlx::query_scalar(
        "UPDATE privacy_incidents SET occurrences = occurrences + 1, last_seen_at = now(), \
            request_ref = $8 \
          WHERE id = (SELECT id FROM privacy_incidents \
                       WHERE environment_id = $1 AND sandbox_id = $2 AND detector = $3 \
                         AND action = $4 AND direction = $5 \
                         AND value_fingerprint IS NOT DISTINCT FROM $6 \
                         AND conversation_id IS NOT DISTINCT FROM $7 \
                         AND created_at > now() - interval '24 hours' \
                       ORDER BY created_at DESC LIMIT 1) \
          RETURNING id",
    )
    .bind(scope.environment_id())
    .bind(incident.sandbox_id)
    .bind(incident.detector.as_str())
    .bind(incident.action.as_str())
    .bind(incident.direction)
    .bind(incident.value_fingerprint)
    .bind(incident.conversation_id)
    .bind(incident.request_ref)
    .fetch_optional(scope.connection())
    .await?;
    if let Some(id) = repeated {
        return Ok(id);
    }

    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO privacy_incidents (environment_id, sandbox_id, detector, action, severity, \
            masked_value, value_fingerprint, destination, cloud_delivery, direction, \
            requested_action, mode, replacement, source, method, path, model, mcp_tool, \
            conversation_id, request_ref, exception_id) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, \
            $17, $18, $19, $20, $21) \
         RETURNING id",
    )
    .bind(scope.environment_id())
    .bind(incident.sandbox_id)
    .bind(incident.detector.as_str())
    .bind(incident.action.as_str())
    .bind(incident.severity)
    .bind(incident.masked_value)
    .bind(incident.value_fingerprint)
    .bind(incident.destination)
    .bind(incident.cloud_delivery)
    .bind(incident.direction)
    .bind(incident.requested_action.as_str())
    .bind(incident.mode)
    .bind(incident.replacement)
    .bind(incident.source.as_str())
    .bind(incident.method)
    .bind(incident.path)
    .bind(incident.model)
    .bind(incident.mcp_tool)
    .bind(incident.conversation_id)
    .bind(incident.request_ref)
    .bind(incident.exception_id)
    .fetch_one(scope.connection())
    .await?;
    Ok(id)
}

/// The most recent incidents of one sandbox, newest first.
///
/// `limit` is bound, and the caller clamps it, so a list request cannot ask for
/// a full table. The index `privacy_incidents_sandbox_idx` is what keeps this
/// cheap on a sandbox with months of traffic.
pub async fn list_incidents(
    scope: &mut TenantScope<'_>,
    sandbox_id: Uuid,
    limit: i64,
) -> Result<Vec<PrivacyIncident>, StoreError> {
    let statement = format!(
        "SELECT {INCIDENT_COLUMNS} FROM privacy_incidents \
          WHERE environment_id = $1 AND sandbox_id = $2 \
          ORDER BY created_at DESC, id DESC \
          LIMIT $3"
    );
    let rows = sqlx::query(&statement)
        .bind(scope.environment_id())
        .bind(sandbox_id)
        .bind(limit)
        .fetch_all(scope.connection())
        .await?;

    let mut incidents = Vec::with_capacity(rows.len());
    for row in rows {
        incidents.push(incident_from_row(&row)?);
    }
    Ok(incidents)
}

/// One incident row, mapped to the domain type.
///
/// Shared by every reader so a new column is one mapping and not four. A
/// spelling the code does not know is an error: silently dropping it would make
/// the log disagree with the policy that produced it.
fn incident_from_row(row: &sqlx::postgres::PgRow) -> Result<PrivacyIncident, StoreError> {
    let detector: String = row.try_get("detector")?;
    let detector = Detector::parse(&detector)
        .ok_or_else(|| StoreError::UnexpectedData(format!("unknown detector: {detector}")))?;
    let action: String = row.try_get("action")?;
    let action = IncidentAction::parse(&action)
        .ok_or_else(|| StoreError::UnexpectedData(format!("unknown incident action: {action}")))?;
    let requested: Option<String> = row.try_get("requested_action")?;
    let requested_action = match requested.as_deref() {
        Some(value) => Some(IncidentAction::parse(value).ok_or_else(|| {
            StoreError::UnexpectedData(format!("unknown requested action: {value}"))
        })?),
        None => None,
    };
    let source: String = row.try_get("source")?;
    let source = IncidentSource::parse(&source)
        .ok_or_else(|| StoreError::UnexpectedData(format!("unknown incident source: {source}")))?;
    Ok(PrivacyIncident {
        id: row.try_get("id")?,
        environment_id: row.try_get("environment_id")?,
        sandbox_id: row.try_get("sandbox_id")?,
        detector,
        action,
        severity: row.try_get("severity")?,
        masked_value: row.try_get("masked_value")?,
        value_fingerprint: row.try_get("value_fingerprint")?,
        destination: row.try_get("destination")?,
        cloud_delivery: row.try_get("cloud_delivery")?,
        direction: row.try_get("direction")?,
        requested_action,
        mode: row.try_get("mode")?,
        replacement: row.try_get("replacement")?,
        source: source.as_str().to_string(),
        method: row.try_get("method")?,
        path: row.try_get("path")?,
        model: row.try_get("model")?,
        mcp_tool: row.try_get("mcp_tool")?,
        conversation_id: row.try_get("conversation_id")?,
        request_ref: row.try_get("request_ref")?,
        exception_id: row.try_get("exception_id")?,
        occurrences: row.try_get("occurrences")?,
        last_seen_at: row.try_get("last_seen_at")?,
        created_at: row.try_get("created_at")?,
    })
}

/// Delete this environment's incidents older than `cutoff` and report how many.
///
/// Retention, not a user delete: the caller passes the moment the window
/// started, derived from `retention_days`. A purge that a second request repeats
/// removes nothing the second time, so it is safe to run on a schedule.
pub async fn purge_before(
    scope: &mut TenantScope<'_>,
    cutoff: DateTime<Utc>,
) -> Result<u64, StoreError> {
    let result = sqlx::query(
        "DELETE FROM privacy_incidents WHERE environment_id = $1 AND created_at < $2",
    )
    .bind(scope.environment_id())
    .bind(cutoff)
    .execute(scope.connection())
    .await?;
    Ok(result.rows_affected())
}


// ---------------------------------------------------------------------------
// DLP across sandboxes: a dashboard and a log
// ---------------------------------------------------------------------------

/// How a DLP breakdown is grouped. A closed list: the name becomes SQL text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DlpGroup {
    Detector,
    Action,
    Severity,
    Sandbox,
    Day,
    Destination,
}

impl DlpGroup {
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "detector" => Self::Detector,
            "action" => Self::Action,
            "severity" => Self::Severity,
            "sandbox" => Self::Sandbox,
            "day" => Self::Day,
            "destination" => Self::Destination,
            _ => return None,
        })
    }

    fn expression(self) -> &'static str {
        match self {
            Self::Detector => "i.detector",
            Self::Action => "i.action",
            Self::Severity => "i.severity",
            Self::Sandbox => "s.name",
            Self::Day => "to_char(i.created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD')",
            Self::Destination => "coalesce(i.destination, 'unknown')",
        }
    }
}

/// One line of the DLP dashboard.
#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct DlpRow {
    pub key: String,
    /// Rows: one per distinct finding.
    pub incidents: i64,
    /// Times the finding was seen: a repeated value counts again.
    pub seen: i64,
    pub blocked: i64,
    pub redacted: i64,
    pub replaced: i64,
    pub allowed: i64,
    /// Incidents that counted as a delivery to a cloud destination.
    pub cloud: i64,
}

/// Incidents over a window, grouped. `owner` narrows to one person's sandboxes.
pub async fn dlp_breakdown(
    scope: &mut TenantScope<'_>,
    group: DlpGroup,
    sandbox_id: Option<Uuid>,
    owner: Option<Uuid>,
    since: DateTime<Utc>,
) -> Result<Vec<DlpRow>, StoreError> {
    let statement = format!(
        "SELECT {key} AS key, count(*)::bigint AS incidents, \
            coalesce(sum(i.occurrences), 0)::bigint AS seen, \
            count(*) FILTER (WHERE i.action = 'blocked')::bigint AS blocked, \
            count(*) FILTER (WHERE i.action = 'redacted')::bigint AS redacted, \
            count(*) FILTER (WHERE i.action = 'replaced')::bigint AS replaced, \
            count(*) FILTER (WHERE i.action = 'allowed')::bigint AS allowed, \
            count(*) FILTER (WHERE i.cloud_delivery)::bigint AS cloud \
           FROM privacy_incidents i \
           JOIN sandboxes s ON s.id = i.sandbox_id \
          WHERE i.environment_id = $1 AND i.created_at >= $2 \
            AND ($3::uuid IS NULL OR i.sandbox_id = $3) \
            AND ($4::uuid IS NULL OR s.owner_user_id = $4) \
          GROUP BY 1 ORDER BY incidents DESC, 1 LIMIT 500",
        key = group.expression()
    );
    let rows = sqlx::query(&statement)
        .bind(scope.environment_id())
        .bind(since)
        .bind(sandbox_id)
        .bind(owner)
        .fetch_all(scope.connection())
        .await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(DlpRow {
            key: row.try_get("key")?,
            incidents: row.try_get("incidents")?,
            seen: row.try_get("seen")?,
            blocked: row.try_get("blocked")?,
            redacted: row.try_get("redacted")?,
            replaced: row.try_get("replaced")?,
            allowed: row.try_get("allowed")?,
            cloud: row.try_get("cloud")?,
        });
    }
    Ok(out)
}

/// One incident with the sandbox it belongs to, for the DLP log.
///
/// The sandbox's name is on the incident row's side; everything from `owner_name`
/// down comes from the joins and is `None` when there was nothing to join: an
/// incident the egress broker reported without a `request_ref` has no turn, and a
/// sandbox without an owner has no user row.
#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct DlpEvent {
    pub incident: PrivacyIncident,
    pub sandbox_name: String,
    /// The sandbox owner's display name.
    pub owner_name: Option<String>,
    /// The agent the request came from (`claude-code`, `codex`, …).
    pub agent: Option<String>,
    /// `account`, `key` or `unknown`: how the request authenticated upstream.
    pub account_kind: Option<String>,
    /// The provider the turn was routed to.
    pub provider: Option<String>,
    /// The model that actually answered, which may differ from the requested one.
    pub served_model: Option<String>,
    /// The address the request went out from.
    pub upstream_ip: Option<String>,
    /// The address a different exit would have produced, for the route chain.
    pub upstream_ip_would: Option<String>,
}

/// The newest incidents in a window, across sandboxes, with filters.
///
/// Masked values only: the value a detector matched is never stored, so this can
/// only ever return the excerpt the incident row holds.
pub async fn dlp_events(
    scope: &mut TenantScope<'_>,
    query: DlpLogQuery<'_>,
) -> Result<Vec<DlpEvent>, StoreError> {
    // Qualified by hand: `id`, `environment_id` and `created_at` exist on several
    // of these tables, so an unqualified list would be ambiguous.
    //
    // The turn join is `LATERAL` with a `LIMIT 1` rather than a plain
    // `LEFT JOIN`: one `request_ref` can have more than one turn (a retried
    // stream), and a plain join would repeat the incident once per turn in the
    // log. The newest turn is the one that describes the request.
    let statement = "SELECT i.id, i.environment_id, i.sandbox_id, i.detector, i.action, \
            i.severity, i.masked_value, i.value_fingerprint, i.destination, i.cloud_delivery, \
            i.direction, i.requested_action, i.mode, i.replacement, i.source, i.method, \
            i.path, i.model, i.mcp_tool, i.conversation_id, i.occurrences, i.last_seen_at, \
            i.created_at, i.request_ref, i.exception_id, s.name AS sandbox_name, \
            u.display_name AS owner_name, t.agent, t.account_kind, t.provider, \
            t.served_model, t.upstream_ip, t.upstream_ip_would \
       FROM privacy_incidents i \
       JOIN sandboxes s ON s.id = i.sandbox_id \
       LEFT JOIN users u ON u.id = s.owner_user_id \
       LEFT JOIN LATERAL ( \
            SELECT c.agent, c.account_kind, c.provider, c.served_model, c.upstream_ip, \
                   c.upstream_ip_would \
              FROM chat_turns c \
             WHERE c.environment_id = i.environment_id AND c.request_ref = i.request_ref \
             ORDER BY c.ts DESC LIMIT 1 \
       ) t ON true \
      WHERE i.environment_id = $1 AND i.created_at >= $2 \
        AND ($3::uuid IS NULL OR i.sandbox_id = $3) \
        AND ($4::uuid IS NULL OR s.owner_user_id = $4) \
        AND ($5::text IS NULL OR i.action = $5) \
        AND ($6::text IS NULL OR i.detector = $6) \
        AND ($7::text IS NULL OR i.direction = $7) \
        AND ($8::text IS NULL OR i.source = $8) \
        AND ($9::text IS NULL \
             OR i.detector ILIKE '%' || $9 || '%' \
             OR s.name ILIKE '%' || $9 || '%' \
             OR coalesce(i.destination, '') ILIKE '%' || $9 || '%' \
             OR coalesce(i.model, '') ILIKE '%' || $9 || '%' \
             OR coalesce(t.served_model, '') ILIKE '%' || $9 || '%' \
             OR coalesce(u.display_name, '') ILIKE '%' || $9 || '%') \
      ORDER BY i.created_at DESC, i.id DESC LIMIT $10";
    let rows = sqlx::query(statement)
        .bind(scope.environment_id())
        .bind(query.since)
        .bind(query.sandbox_id)
        .bind(query.owner)
        .bind(query.action)
        .bind(query.detector)
        .bind(query.direction)
        .bind(query.source)
        .bind(query.q)
        .bind(query.limit.clamp(1, 500))
        .fetch_all(scope.connection())
        .await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(DlpEvent {
            sandbox_name: row.try_get("sandbox_name")?,
            owner_name: row.try_get("owner_name")?,
            agent: row.try_get("agent")?,
            account_kind: row.try_get("account_kind")?,
            provider: row.try_get("provider")?,
            served_model: row.try_get("served_model")?,
            upstream_ip: row.try_get("upstream_ip")?,
            upstream_ip_would: row.try_get("upstream_ip_would")?,
            incident: incident_from_row(&row)?,
        });
    }
    Ok(out)
}

/// The DLP log's filters, so the signature stays readable as it grows.
pub struct DlpLogQuery<'a> {
    pub sandbox_id: Option<Uuid>,
    /// Narrow to one owner's sandboxes. `None` means the whole environment.
    pub owner: Option<Uuid>,
    pub since: DateTime<Utc>,
    pub action: Option<&'a str>,
    pub detector: Option<&'a str>,
    pub direction: Option<&'a str>,
    pub source: Option<&'a str>,
    /// A free-text needle over detector, sandbox, destination, model, served
    /// model and owner. Case-insensitive; `None` does not filter.
    pub q: Option<&'a str>,
    pub limit: i64,
}

/// One entry of the policy history: an audit event about DLP settings.
#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct DlpChange {
    pub id: Uuid,
    pub kind: String,
    pub actor_label: String,
    /// `sandbox` or `environment`: which row the change applied to.
    pub subject_type: Option<String>,
    pub subject_id: Option<Uuid>,
    /// The stored detail: the new values, and for a save since migration 0079
    /// the old ones under `before`.
    pub detail: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

/// The DLP policy history for one sandbox and the default it inherits.
///
/// Read from the audit trail rather than a table of its own: a change to the
/// policy is already an audited act, and a second copy would be a second truth.
/// A sandbox's own changes are selected by its id; the environment default by
/// the environment's id with `subject_type = 'environment'`.
pub async fn dlp_changes(
    scope: &mut TenantScope<'_>,
    sandbox_id: Option<Uuid>,
    environment_id: Uuid,
    limit: i64,
) -> Result<Vec<DlpChange>, StoreError> {
    let statement = "SELECT id, kind, actor_label, subject_type, subject_id, detail, created_at \
           FROM audit_events \
          WHERE environment_id = $1 AND kind LIKE 'privacy.%' \
            AND (($2::uuid IS NOT NULL AND subject_type = 'sandbox' AND subject_id = $2) \
                 OR subject_type = 'environment' AND subject_id = $3) \
          ORDER BY created_at DESC, id DESC LIMIT $4";
    let rows = sqlx::query(statement)
        .bind(environment_id)
        .bind(sandbox_id)
        .bind(environment_id)
        .bind(limit.clamp(1, 500))
        .fetch_all(scope.connection())
        .await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(DlpChange {
            id: row.try_get("id")?,
            kind: row.try_get("kind")?,
            actor_label: row.try_get("actor_label")?,
            subject_type: row.try_get("subject_type")?,
            subject_id: row.try_get("subject_id")?,
            detail: row.try_get("detail")?,
            created_at: row.try_get("created_at")?,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod dlp_group_tests {
    use super::DlpGroup;

    #[test]
    fn every_group_has_a_key_and_nothing_else_does() {
        for (key, group) in [
            ("detector", DlpGroup::Detector),
            ("action", DlpGroup::Action),
            ("severity", DlpGroup::Severity),
            ("sandbox", DlpGroup::Sandbox),
            ("day", DlpGroup::Day),
            ("destination", DlpGroup::Destination),
        ] {
            assert_eq!(DlpGroup::parse(key), Some(group), "{key}");
        }
        for wrong in ["", "model", "detector; drop table privacy_incidents"] {
            assert_eq!(DlpGroup::parse(wrong), None, "{wrong}");
        }
    }
}
