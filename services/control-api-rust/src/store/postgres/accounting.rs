//! Accounting rows, price list, daily rollup, and retention settings
//! (`0058_accounting.sql`).
//!
//! One aggregate, one file: a request row is priced from the same price list the
//! rollup was built from, and the retention setting decides how long the raw
//! rows survive. Splitting them would put the price lookup and the row it prices
//! in two files that have to agree.
//!
//! Three rules the SQL keeps, rather than the caller:
//!
//! 1. **A cost is `NULL` or a number.** `sum(cost_micros)` is used as-is, so a
//!    bucket with no priced request stores `NULL` instead of a zero that reads as
//!    "free". The unpriced count is read next to the sum, so a partial total
//!    cannot pass for a complete one.
//! 2. **Every statement names its columns and filters by `environment_id`.**
//!    Row-level security repeats the filter, so a forgotten bind in a new
//!    statement returns nothing instead of another tenant's rows.
//! 3. **A summary outlives its sandbox.** `summary_by_sandbox` reads the name
//!    from `sandboxes` and falls back to the id; a deleted sandbox still appears
//!    in an old window instead of vanishing from it.
//!
//! `model_prices` is the one table whose read crosses the tenant on purpose: a
//! global row (`environment_id IS NULL`) is the fallback every environment
//! shares, which is exactly what the policy in the migration allows.

use chrono::{DateTime, Utc};
use sqlx::Row;
use uuid::Uuid;

use super::super::TenantScope;
use crate::domain::accounting::{
    AccountingRow, AccountingSettings, AccountingSummary, Direction, NewAccountingRow, Price,
    PriceSource, PrivacyStat, SandboxSummary, Totals, Usage,
};
use crate::error::StoreError;

/// The request columns, named so a schema change is a mapping error rather than
/// a silently swapped field.
const REQUEST_COLUMNS: &str = "id, environment_id, sandbox_id, agent_instance, provider, model, \
     direction, endpoint, status, latency_ms, prompt_tokens, completion_tokens, cached_tokens, \
     reasoning_tokens, content_digest, cost_micros, price_source, privacy_action, \
     privacy_detector, created_at";

/// The largest page a caller can read, enforced by the service, repeated here as
/// the SQL bound so a direct caller cannot ask for the table.
const MAX_LIMIT: i64 = 1_000;

/// How many groups a summary returns. A window with more distinct model names
/// than this is a data problem, not a screen; the totals row still covers every
/// request because it is its own statement.
const MAX_GROUPS: i64 = 200;

/// Fit a token count into the column's `int` range.
///
/// A cast would wrap a count above `i32::MAX` into a negative number and turn a
/// write into a constraint failure; saturating keeps the row and the wrong-by-a-
/// billion number is still a number the arithmetic can reason about.
fn token_count(value: i64) -> i32 {
    value.clamp(0, i32::MAX as i64) as i32
}

/// Whether a sandbox exists in this environment.
///
/// The accounting write path must not create a row for a sandbox that does not
/// exist. It lives here rather than in `sandboxes.rs` because it exists only for
/// this path, the same reason the ingest node lookup lives in `telemetry.rs`.
pub async fn sandbox_exists(
    scope: &mut TenantScope<'_>,
    sandbox_id: Uuid,
) -> Result<bool, StoreError> {
    let found: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM sandboxes WHERE environment_id = $1 AND id = $2")
            .bind(scope.environment_id())
            .bind(sandbox_id)
            .fetch_optional(scope.connection())
            .await?;
    Ok(found.is_some())
}

/// Write one request row and return its id.
///
/// The cost is an argument rather than a field of [`NewAccountingRow`] because
/// the store does not price anything: the arithmetic belongs to
/// `domain::accounting`, and a repository that computed it would be a second
/// place where a token's price is decided.
///
/// The row is evidence, so there is no update statement for it: a correction is
/// a new row, and the retention sweep is the only delete. That is what makes a
/// stored cost reproducible — nothing can rewrite the tokens it was derived
/// from.
pub async fn insert_request(
    scope: &mut TenantScope<'_>,
    request: &NewAccountingRow,
    cost_micros: Option<i64>,
    price_source: PriceSource,
) -> Result<Uuid, StoreError> {
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO accounting_requests (environment_id, sandbox_id, agent_instance, provider, \
            model, direction, endpoint, status, latency_ms, prompt_tokens, completion_tokens, \
            cached_tokens, reasoning_tokens, content_digest, cost_micros, price_source, \
            privacy_action, privacy_detector, detail) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, \
            $17, $18, $19) \
         RETURNING id",
    )
    .bind(scope.environment_id())
    .bind(request.sandbox_id)
    .bind(&request.agent_instance)
    .bind(&request.provider)
    .bind(&request.model)
    .bind(request.direction.as_str())
    .bind(&request.endpoint)
    .bind(request.status)
    .bind(request.latency_ms)
    .bind(token_count(request.usage.prompt))
    .bind(token_count(request.usage.completion))
    .bind(token_count(request.usage.cached))
    .bind(token_count(request.usage.reasoning))
    .bind(&request.content_digest)
    .bind(cost_micros)
    .bind(price_source.as_str())
    .bind(&request.privacy_action)
    .bind(&request.privacy_detector)
    .bind(&request.detail)
    .fetch_one(scope.connection())
    .await?;
    Ok(id)
}

/// The newest request rows, for the environment or one sandbox.
///
/// `limit` is clamped here as well as in the service: a repository is also called
/// from tests and jobs, and an unbounded list is the kind of query that only
/// hurts once the table is large.
pub async fn list_requests(
    scope: &mut TenantScope<'_>,
    sandbox_id: Option<Uuid>,
    since: DateTime<Utc>,
    limit: i64,
) -> Result<Vec<AccountingRow>, StoreError> {
    let statement = format!(
        "SELECT {REQUEST_COLUMNS} FROM accounting_requests \
          WHERE environment_id = $1 AND created_at >= $2 \
            AND ($3::uuid IS NULL OR sandbox_id = $3) \
          ORDER BY created_at DESC, id DESC \
          LIMIT $4"
    );
    let limit = limit.clamp(1, MAX_LIMIT);
    let rows = sqlx::query(&statement)
        .bind(scope.environment_id())
        .bind(since)
        .bind(sandbox_id)
        .bind(limit)
        .fetch_all(scope.connection())
        .await?;

    let mut requests = Vec::with_capacity(rows.len());
    for row in rows {
        requests.push(map_request(&row)?);
    }
    Ok(requests)
}

/// Map one row to the domain type, refusing a value the domain cannot represent.
fn map_request(row: &sqlx::postgres::PgRow) -> Result<AccountingRow, StoreError> {
    let direction: String = row.try_get("direction")?;
    let direction = Direction::parse(&direction)
        .ok_or_else(|| StoreError::UnexpectedData(format!("unknown direction: {direction}")))?;
    let source: String = row.try_get("price_source")?;
    let price_source = PriceSource::parse(&source)
        .ok_or_else(|| StoreError::UnexpectedData(format!("unknown price source: {source}")))?;

    Ok(AccountingRow {
        id: row.try_get("id")?,
        environment_id: row.try_get("environment_id")?,
        sandbox_id: row.try_get("sandbox_id")?,
        agent_instance: row.try_get("agent_instance")?,
        provider: row.try_get("provider")?,
        model: row.try_get("model")?,
        direction,
        endpoint: row.try_get("endpoint")?,
        status: row.try_get("status")?,
        latency_ms: row.try_get("latency_ms")?,
        usage: Usage {
            prompt: i64::from(row.try_get::<i32, _>("prompt_tokens")?),
            completion: i64::from(row.try_get::<i32, _>("completion_tokens")?),
            cached: i64::from(row.try_get::<i32, _>("cached_tokens")?),
            reasoning: i64::from(row.try_get::<i32, _>("reasoning_tokens")?),
        },
        content_digest: row.try_get("content_digest")?,
        cost_micros: row.try_get("cost_micros")?,
        price_source,
        privacy_action: row.try_get("privacy_action")?,
        privacy_detector: row.try_get("privacy_detector")?,
        created_at: row.try_get("created_at")?,
    })
}

/// A window grouped by model, most expensive first.
///
/// `count(cost_micros)` is not in the row: the group's `cost_micros` is `NULL`
/// when nothing in it was priced, which is the same information without a second
/// column the caller has to know how to read.
pub async fn summary_by_model(
    scope: &mut TenantScope<'_>,
    sandbox_id: Option<Uuid>,
    since: DateTime<Utc>,
) -> Result<Vec<AccountingSummary>, StoreError> {
    let rows = sqlx::query(
        "SELECT coalesce(model, 'unknown') AS model, \
                count(*)::bigint AS requests, \
                coalesce(sum(prompt_tokens), 0)::bigint AS prompt_tokens, \
                coalesce(sum(completion_tokens), 0)::bigint AS completion_tokens, \
                coalesce(sum(cached_tokens), 0)::bigint AS cached_tokens, \
                sum(cost_micros)::bigint AS cost_micros \
           FROM accounting_requests \
          WHERE environment_id = $1 AND created_at >= $2 \
            AND ($3::uuid IS NULL OR sandbox_id = $3) \
          GROUP BY 1 \
          ORDER BY sum(coalesce(cost_micros, 0)) DESC, count(*) DESC, 1 \
          LIMIT $4",
    )
    .bind(scope.environment_id())
    .bind(since)
    .bind(sandbox_id)
    .bind(MAX_GROUPS)
    .fetch_all(scope.connection())
    .await?;

    let mut summaries = Vec::with_capacity(rows.len());
    for row in rows {
        summaries.push(AccountingSummary {
            model: row.try_get("model")?,
            requests: row.try_get("requests")?,
            prompt_tokens: row.try_get("prompt_tokens")?,
            completion_tokens: row.try_get("completion_tokens")?,
            cached_tokens: row.try_get("cached_tokens")?,
            cost_micros: row.try_get("cost_micros")?,
        });
    }
    Ok(summaries)
}

/// A window grouped by sandbox, for the org-level overview.
///
/// No sandbox filter: the caller asked for the organisation, and the tenant is
/// still enforced by `environment_id` and by row-level security. The name comes
/// from a left join so a sandbox deleted after the window still appears, under
/// its id.
pub async fn summary_by_sandbox(
    scope: &mut TenantScope<'_>,
    since: DateTime<Utc>,
) -> Result<Vec<SandboxSummary>, StoreError> {
    let rows = sqlx::query(
        "SELECT a.sandbox_id, \
                coalesce(s.name, a.sandbox_id::text) AS sandbox_name, \
                count(*)::bigint AS requests, \
                sum(a.cost_micros)::bigint AS cost_micros \
           FROM accounting_requests a \
           LEFT JOIN sandboxes s \
                  ON s.id = a.sandbox_id AND s.environment_id = a.environment_id \
          WHERE a.environment_id = $1 AND a.created_at >= $2 \
          GROUP BY a.sandbox_id, s.name \
          ORDER BY sum(coalesce(a.cost_micros, 0)) DESC, count(*) DESC, a.sandbox_id \
          LIMIT $3",
    )
    .bind(scope.environment_id())
    .bind(since)
    .bind(MAX_GROUPS)
    .fetch_all(scope.connection())
    .await?;

    let mut summaries = Vec::with_capacity(rows.len());
    for row in rows {
        summaries.push(SandboxSummary {
            sandbox_id: row.try_get("sandbox_id")?,
            sandbox_name: row.try_get("sandbox_name")?,
            requests: row.try_get("requests")?,
            cost_micros: row.try_get("cost_micros")?,
        });
    }
    Ok(summaries)
}

/// What the privacy layer did in a window, most frequent first.
///
/// Rows without a verdict are skipped rather than grouped under `none`: "we did
/// not inspect this" is not an action, and a bucket for it would read like one.
pub async fn privacy_stats(
    scope: &mut TenantScope<'_>,
    sandbox_id: Option<Uuid>,
    since: DateTime<Utc>,
) -> Result<Vec<PrivacyStat>, StoreError> {
    let rows = sqlx::query(
        "SELECT privacy_action AS action, count(*)::bigint AS count \
           FROM accounting_requests \
          WHERE environment_id = $1 AND created_at >= $2 \
            AND ($3::uuid IS NULL OR sandbox_id = $3) \
            AND privacy_action IS NOT NULL \
          GROUP BY 1 \
          ORDER BY count(*) DESC, 1 \
          LIMIT $4",
    )
    .bind(scope.environment_id())
    .bind(since)
    .bind(sandbox_id)
    .bind(MAX_GROUPS)
    .fetch_all(scope.connection())
    .await?;

    let mut stats = Vec::with_capacity(rows.len());
    for row in rows {
        stats.push(PrivacyStat {
            action: row.try_get("action")?,
            count: row.try_get("count")?,
        });
    }
    Ok(stats)
}

/// The whole window in one row.
///
/// A separate statement rather than a sum of [`summary_by_model`], because that
/// list is capped: a total assembled from a capped list would quietly describe a
/// subset of the traffic it claims to describe.
pub async fn totals(
    scope: &mut TenantScope<'_>,
    sandbox_id: Option<Uuid>,
    since: DateTime<Utc>,
) -> Result<Totals, StoreError> {
    let row = sqlx::query(
        "SELECT count(*)::bigint AS requests, \
                coalesce(sum(prompt_tokens), 0)::bigint AS prompt_tokens, \
                coalesce(sum(completion_tokens), 0)::bigint AS completion_tokens, \
                coalesce(sum(cached_tokens), 0)::bigint AS cached_tokens, \
                sum(cost_micros)::bigint AS cost_micros, \
                count(*) FILTER (WHERE cost_micros IS NULL)::bigint AS price_missing_requests \
           FROM accounting_requests \
          WHERE environment_id = $1 AND created_at >= $2 \
            AND ($3::uuid IS NULL OR sandbox_id = $3)",
    )
    .bind(scope.environment_id())
    .bind(since)
    .bind(sandbox_id)
    .fetch_one(scope.connection())
    .await?;

    Ok(Totals {
        requests: row.try_get("requests")?,
        prompt_tokens: row.try_get("prompt_tokens")?,
        completion_tokens: row.try_get("completion_tokens")?,
        cached_tokens: row.try_get("cached_tokens")?,
        cost_micros: row.try_get("cost_micros")?,
        price_missing_requests: row.try_get("price_missing_requests")?,
    })
}

/// Read one price list row: an environment's override, or the global fallback.
///
/// `environment_id: None` reads the global row. `IS NOT DISTINCT FROM` is what
/// makes that work: `= NULL` matches nothing, which is the classic reason a
/// fallback silently never applies.
pub async fn model_price(
    scope: &mut TenantScope<'_>,
    environment_id: Option<Uuid>,
    model: &str,
) -> Result<Option<Price>, StoreError> {
    let row = sqlx::query(
        "SELECT input_micros_per_mtok, output_micros_per_mtok, cached_micros_per_mtok \
           FROM model_prices \
          WHERE environment_id IS NOT DISTINCT FROM $1 AND model = $2",
    )
    .bind(environment_id)
    .bind(model)
    .fetch_optional(scope.connection())
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    Ok(Some(Price {
        input_per_mtok: row.try_get("input_micros_per_mtok")?,
        output_per_mtok: row.try_get("output_micros_per_mtok")?,
        cached_per_mtok: row.try_get("cached_micros_per_mtok")?,
    }))
}

/// The price of a model for one environment: its own row first, then the global.
///
/// The order is policy, not convenience: an environment that negotiated a rate
/// must not be priced by the shared list, and an environment that named none must
/// still get a price. Both answers carry [`PriceSource::Catalog`], because both
/// come from the catalog rather than from a provider's own report.
pub async fn price_for_model(
    scope: &mut TenantScope<'_>,
    environment_id: Uuid,
    model: &str,
) -> Result<Option<(Price, PriceSource)>, StoreError> {
    if let Some(price) = model_price(scope, Some(environment_id), model).await? {
        return Ok(Some((price, PriceSource::Catalog)));
    }
    if let Some(price) = model_price(scope, None, model).await? {
        return Ok(Some((price, PriceSource::Catalog)));
    }
    Ok(None)
}

/// Write a price list row for its scope, replacing what was there.
///
/// The conflict target is the named constraint because a global row's key
/// contains a NULL, and inference through a nullable key is only reliable that
/// way. `environment_id: None` is refused by row-level security for the
/// application role; a global price is a migration's or the owner's decision.
pub async fn upsert_price(
    scope: &mut TenantScope<'_>,
    environment_id: Option<Uuid>,
    model: &str,
    price: &Price,
) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO model_prices (environment_id, model, input_micros_per_mtok, \
            output_micros_per_mtok, cached_micros_per_mtok, updated_at) \
         VALUES ($1, $2, $3, $4, $5, now()) \
         ON CONFLICT ON CONSTRAINT model_prices_scope DO UPDATE SET \
            input_micros_per_mtok = EXCLUDED.input_micros_per_mtok, \
            output_micros_per_mtok = EXCLUDED.output_micros_per_mtok, \
            cached_micros_per_mtok = EXCLUDED.cached_micros_per_mtok, \
            updated_at = now()",
    )
    .bind(environment_id)
    .bind(model)
    .bind(price.input_per_mtok)
    .bind(price.output_per_mtok)
    .bind(price.cached_per_mtok)
    .execute(scope.connection())
    .await?;
    Ok(())
}

/// Fold raw rows since `since` into `accounting_daily` and report the rows
/// written.
///
/// The upsert overwrites the affected buckets instead of adding to them, so a
/// re-run over the same window produces the same rows. That is what lets the
/// purge job call it before deleting raw rows and stay safe when two runs
/// overlap.
///
/// `requests` is an `int`, so the count is capped rather than cast: a wrapped
/// negative count would turn a busy day into a credit.
pub async fn rollup_daily(
    scope: &mut TenantScope<'_>,
    since: DateTime<Utc>,
) -> Result<u64, StoreError> {
    let result = sqlx::query(
        "INSERT INTO accounting_daily (environment_id, sandbox_id, model, day, requests, \
            prompt_tokens, completion_tokens, cached_tokens, cost_micros, cache_write_tokens, \
            estimated_cost_micros) \
         SELECT environment_id, sandbox_id, coalesce(model, 'unknown'), \
                (created_at AT TIME ZONE 'UTC')::date, \
                least(count(*), 2147483647)::int, \
                coalesce(sum(prompt_tokens), 0)::bigint, \
                coalesce(sum(completion_tokens), 0)::bigint, \
                coalesce(sum(cached_tokens), 0)::bigint, \
                sum(cost_micros)::bigint, \
                coalesce(sum(cache_write_tokens), 0)::bigint, \
                sum(estimated_cost_micros)::bigint \
           FROM accounting_requests \
          WHERE environment_id = $1 AND created_at >= $2 \
          GROUP BY 1, 2, 3, 4 \
         ON CONFLICT (environment_id, sandbox_id, model, day) DO UPDATE SET \
            requests = EXCLUDED.requests, \
            prompt_tokens = EXCLUDED.prompt_tokens, \
            completion_tokens = EXCLUDED.completion_tokens, \
            cached_tokens = EXCLUDED.cached_tokens, \
            cost_micros = EXCLUDED.cost_micros, \
            cache_write_tokens = EXCLUDED.cache_write_tokens, \
            estimated_cost_micros = EXCLUDED.estimated_cost_micros",
    )
    .bind(scope.environment_id())
    .bind(since)
    .execute(scope.connection())
    .await?;
    Ok(result.rows_affected())
}

/// The environment's retention setting, or the domain default when it never
/// saved one.
///
/// "Never configured" and "configured to the default" are the same policy, so a
/// missing row is not an error and the caller never has to invent the number.
pub async fn get_settings(scope: &mut TenantScope<'_>) -> Result<AccountingSettings, StoreError> {
    let row = sqlx::query(
        "SELECT retention_days, keep_daily, updated_by, updated_at \
           FROM accounting_settings WHERE environment_id = $1",
    )
    .bind(scope.environment_id())
    .fetch_optional(scope.connection())
    .await?;
    let Some(row) = row else {
        return Ok(AccountingSettings::default());
    };
    Ok(AccountingSettings {
        retention_days: row.try_get("retention_days")?,
        keep_daily: row.try_get("keep_daily")?,
        updated_by: row.try_get("updated_by")?,
        updated_at: Some(row.try_get("updated_at")?),
    })
}

/// Write the retention setting for this environment.
///
/// One row per environment, so the environment is the conflict target and the
/// update replaces both fields: a save is the whole setting, not a patch that
/// could leave half of it stale.
pub async fn save_settings(
    scope: &mut TenantScope<'_>,
    settings: &AccountingSettings,
    updated_by: Uuid,
) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO accounting_settings (environment_id, retention_days, keep_daily, \
            updated_by, updated_at) \
         VALUES ($1, $2, $3, $4, now()) \
         ON CONFLICT (environment_id) DO UPDATE SET \
            retention_days = EXCLUDED.retention_days, \
            keep_daily = EXCLUDED.keep_daily, \
            updated_by = EXCLUDED.updated_by, \
            updated_at = now()",
    )
    .bind(scope.environment_id())
    .bind(settings.retention_days)
    .bind(settings.keep_daily)
    .bind(updated_by)
    .execute(scope.connection())
    .await?;
    Ok(())
}

/// Delete this environment's raw request rows older than `cutoff`.
///
/// Raw rows only: `accounting_daily` is the rollup that is kept, and deleting it
/// here would turn a retention sweep into a history loss. A second run removes
/// nothing the first one did, so it is safe on a schedule.
pub async fn purge_before(
    scope: &mut TenantScope<'_>,
    cutoff: DateTime<Utc>,
) -> Result<u64, StoreError> {
    let result =
        sqlx::query("DELETE FROM accounting_requests WHERE environment_id = $1 AND created_at < $2")
            .bind(scope.environment_id())
            .bind(cutoff)
            .execute(scope.connection())
            .await?;
    Ok(result.rows_affected())
}


// ---------------------------------------------------------------------------
// Usage observed by the egress proxy, and the breakdown over everything
// ---------------------------------------------------------------------------

/// One observed model call, ready to store.
pub struct ObservedRow<'a> {
    /// The traffic event this call was read from: a retried batch inserts the same
    /// row, not a second one.
    pub id: Uuid,
    pub sandbox_id: Uuid,
    pub provider: &'a str,
    pub model: Option<&'a str>,
    pub endpoint: Option<&'a str>,
    pub status: i32,
    pub latency_ms: i32,
    pub usage: crate::domain::accounting::ObservedUsage,
    pub cache_markers: i32,
    pub auth: crate::domain::accounting::AuthKind,
    pub conversation: Option<&'a str>,
    /// Set for a call made with an API key; `None` for an account or no price.
    pub cost_micros: Option<i64>,
    /// The list-price estimate; only for account calls.
    pub estimated_cost_micros: Option<i64>,
    pub price_source: PriceSource,
}

/// Store one call the egress proxy read. Tokens are normalised first, so the
/// table's CHECKs describe the row and are not what discovers a bad one.
pub async fn insert_observed(
    scope: &mut TenantScope<'_>,
    row: &ObservedRow<'_>,
) -> Result<bool, StoreError> {
    let usage = row.usage.normalised();
    let inserted: Option<Uuid> = sqlx::query_scalar(
        "INSERT INTO accounting_requests (id, environment_id, sandbox_id, provider, model, direction, \
            endpoint, status, latency_ms, prompt_tokens, completion_tokens, cached_tokens, \
            reasoning_tokens, cost_micros, price_source, cache_write_tokens, cache_write_1h_tokens, \
            cache_markers, auth, source, conversation, estimated_cost_micros) \
         VALUES ($20, $1, $2, $3, $4, 'out', $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, \
            $17, 'egress', $18, $19) \
         ON CONFLICT (id) DO NOTHING \
         RETURNING id",
    )
    .bind(scope.environment_id())
    .bind(row.sandbox_id)
    .bind(row.provider)
    .bind(row.model)
    .bind(row.endpoint)
    .bind(row.status)
    .bind(row.latency_ms)
    .bind(token_count(usage.prompt()))
    .bind(token_count(usage.output))
    .bind(token_count(usage.cache_read))
    .bind(token_count(usage.reasoning))
    .bind(row.cost_micros)
    .bind(row.price_source.as_str())
    .bind(token_count(usage.cache_write))
    .bind(token_count(usage.cache_write_1h))
    .bind(row.cache_markers.max(0))
    .bind(row.auth.as_str())
    .bind(row.conversation)
    .bind(row.estimated_cost_micros)
    .bind(row.id)
    .fetch_optional(scope.connection())
    .await?;
    Ok(inserted.is_some())
}

/// How a breakdown is grouped. A closed list: the key becomes SQL text, so only a
/// name this function knows may pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsageGroup {
    Provider,
    Model,
    Sandbox,
    Auth,
    Day,
    Owner,
    Site,
    Organization,
    Agent,
    Conversation,
}

impl UsageGroup {
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "provider" => Self::Provider,
            "model" => Self::Model,
            "sandbox" => Self::Sandbox,
            "auth" => Self::Auth,
            "day" => Self::Day,
            "owner" => Self::Owner,
            "site" => Self::Site,
            "organization" => Self::Organization,
            "agent" => Self::Agent,
            "conversation" => Self::Conversation,
            _ => return None,
        })
    }

    fn expression(self) -> &'static str {
        match self {
            Self::Provider => "coalesce(a.provider, 'unknown')",
            Self::Model => "coalesce(a.model, 'unknown')",
            Self::Sandbox => "s.name",
            Self::Auth => "a.auth",
            Self::Day => "to_char(a.created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD')",
            Self::Owner => "coalesce(u.email, 'unassigned')",
            Self::Site => "coalesce(st.name, n.region, 'unplaced')",
            Self::Organization => "coalesce(o.name, 'unknown')",
            Self::Agent => "coalesce(a.agent_instance, 'unknown')",
            Self::Conversation => "coalesce(a.conversation, 'none')",
        }
    }
}

/// One line of a breakdown.
#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct UsageRow {
    pub key: String,
    pub requests: i64,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub cache_write_1h_tokens: i64,
    pub reasoning_tokens: i64,
    pub cache_markers: i64,
    /// What API-key calls cost; `None` when none had a price.
    pub cost_micros: Option<i64>,
    /// What account calls would cost at list price. Never added to `cost_micros`.
    pub estimated_cost_micros: Option<i64>,
    /// Calls with neither a cost nor an estimate.
    pub unpriced_requests: i64,
    pub account_requests: i64,
    pub key_requests: i64,
    /// Cache reads as a share of everything read (0..=1).
    pub cache_hit_rate: f64,
}

/// Usage over a window, grouped.
///
/// `owner` narrows to one user's sandboxes (a non-admin sees only their own).
pub async fn usage_breakdown(
    scope: &mut TenantScope<'_>,
    group: UsageGroup,
    sandbox_id: Option<Uuid>,
    owner: Option<Uuid>,
    since: DateTime<Utc>,
) -> Result<Vec<UsageRow>, StoreError> {
    let statement = format!(
        "SELECT {key} AS key, \
            count(*)::bigint AS requests, \
            coalesce(sum(a.prompt_tokens), 0)::bigint AS prompt, \
            coalesce(sum(a.completion_tokens), 0)::bigint AS completion, \
            coalesce(sum(a.cached_tokens), 0)::bigint AS cache_read, \
            coalesce(sum(a.cache_write_tokens), 0)::bigint AS cache_write, \
            coalesce(sum(a.cache_write_1h_tokens), 0)::bigint AS cache_write_1h, \
            coalesce(sum(a.reasoning_tokens), 0)::bigint AS reasoning, \
            coalesce(sum(a.cache_markers), 0)::bigint AS markers, \
            sum(a.cost_micros)::bigint AS cost, \
            sum(a.estimated_cost_micros)::bigint AS estimated, \
            count(*) FILTER (WHERE a.cost_micros IS NULL AND a.estimated_cost_micros IS NULL \
                             AND a.prompt_tokens + a.completion_tokens > 0)::bigint AS unpriced, \
            count(*) FILTER (WHERE a.auth = 'account')::bigint AS account_requests, \
            count(*) FILTER (WHERE a.auth = 'key')::bigint AS key_requests \
           FROM accounting_requests a \
           JOIN sandboxes s ON s.id = a.sandbox_id \
           LEFT JOIN nodes n ON n.id = s.node_id \
           LEFT JOIN sites st ON st.id = n.site_id \
           LEFT JOIN environments e ON e.id = a.environment_id \
           LEFT JOIN organizations o ON o.id = e.organization_id \
           LEFT JOIN users u ON u.id = s.owner_user_id \
          WHERE a.environment_id = $1 AND a.created_at >= $2 \
            AND ($3::uuid IS NULL OR a.sandbox_id = $3) \
            AND ($4::uuid IS NULL OR s.owner_user_id = $4) \
            AND a.prompt_tokens + a.completion_tokens > 0 \
          GROUP BY 1 \
          ORDER BY sum(a.prompt_tokens + a.completion_tokens) DESC, 1 \
          LIMIT 500",
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
        let prompt: i64 = row.try_get("prompt")?;
        let cache_read: i64 = row.try_get("cache_read")?;
        out.push(UsageRow {
            key: row.try_get("key")?,
            requests: row.try_get("requests")?,
            prompt_tokens: prompt,
            completion_tokens: row.try_get("completion")?,
            cache_read_tokens: cache_read,
            cache_write_tokens: row.try_get("cache_write")?,
            cache_write_1h_tokens: row.try_get("cache_write_1h")?,
            reasoning_tokens: row.try_get("reasoning")?,
            cache_markers: row.try_get("markers")?,
            cost_micros: row.try_get("cost")?,
            estimated_cost_micros: row.try_get("estimated")?,
            unpriced_requests: row.try_get("unpriced")?,
            account_requests: row.try_get("account_requests")?,
            key_requests: row.try_get("key_requests")?,
            cache_hit_rate: if prompt > 0 { cache_read as f64 / prompt as f64 } else { 0.0 },
        });
    }
    Ok(out)
}


/// One price-list entry as the console shows it.
#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct PriceEntry {
    pub model: String,
    pub input_micros_per_mtok: i64,
    pub output_micros_per_mtok: i64,
    pub cached_micros_per_mtok: Option<i64>,
    /// `environment` or `global`.
    pub scope: String,
}

/// Every price the environment prices by: its own rows and the global ones.
pub async fn list_prices(scope: &mut TenantScope<'_>) -> Result<Vec<PriceEntry>, StoreError> {
    let rows = sqlx::query(
        "SELECT model, input_micros_per_mtok, output_micros_per_mtok, cached_micros_per_mtok, \
                (environment_id IS NULL) AS global \
           FROM model_prices WHERE environment_id = $1 OR environment_id IS NULL \
          ORDER BY model, global",
    )
    .bind(scope.environment_id())
    .fetch_all(scope.connection())
    .await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let global: bool = row.try_get("global")?;
        out.push(PriceEntry {
            model: row.try_get("model")?,
            input_micros_per_mtok: row.try_get("input_micros_per_mtok")?,
            output_micros_per_mtok: row.try_get("output_micros_per_mtok")?,
            cached_micros_per_mtok: row.try_get("cached_micros_per_mtok")?,
            scope: if global { "global" } else { "environment" }.into(),
        });
    }
    Ok(out)
}

/// Models that were used but have no price, with how many calls and tokens.
pub async fn unpriced_models(
    scope: &mut TenantScope<'_>,
    since: DateTime<Utc>,
) -> Result<Vec<(String, i64, i64)>, StoreError> {
    let rows = sqlx::query(
        "SELECT a.model, count(*)::bigint AS calls, \
                coalesce(sum(a.prompt_tokens + a.completion_tokens), 0)::bigint AS tokens \
           FROM accounting_requests a \
          WHERE a.environment_id = $1 AND a.created_at >= $2 AND a.model IS NOT NULL \
            AND a.cost_micros IS NULL AND a.estimated_cost_micros IS NULL \
            AND a.prompt_tokens + a.completion_tokens > 0 \
          GROUP BY a.model ORDER BY tokens DESC LIMIT 100",
    )
    .bind(scope.environment_id())
    .bind(since)
    .fetch_all(scope.connection())
    .await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push((row.try_get("model")?, row.try_get("calls")?, row.try_get("tokens")?));
    }
    Ok(out)
}

/// Price the calls of `model` that were stored without one, now that it has a price.
///
/// A call made with an API key (and anything the accounting proxy wrote) gets a
/// cost; a call made through an account gets the list-price estimate. Rows that
/// already have a figure are left alone: a later price change is not applied
/// backwards over numbers someone may already have reported.
pub async fn reprice_model(
    scope: &mut TenantScope<'_>,
    model: &str,
    price: &Price,
) -> Result<u64, StoreError> {
    let result = sqlx::query(
        "WITH priced AS ( \
            SELECT id, auth, source, round(( \
                (prompt_tokens - cached_tokens - cache_write_tokens)::numeric * $3::numeric \
              + cached_tokens::numeric * $5::numeric \
              + (cache_write_tokens - cache_write_1h_tokens)::numeric * $3::numeric * 5 / 4 \
              + cache_write_1h_tokens::numeric * $3::numeric * 2 \
              + completion_tokens::numeric * $4::numeric) / 1000000)::bigint AS figure \
              FROM accounting_requests \
             WHERE environment_id = $1 AND model = $2 \
               AND cost_micros IS NULL AND estimated_cost_micros IS NULL \
               AND prompt_tokens + completion_tokens > 0) \
         UPDATE accounting_requests a SET \
            cost_micros = CASE WHEN p.auth = 'key' OR p.source = 'accounting-proxy' THEN p.figure END, \
            estimated_cost_micros = CASE WHEN p.auth <> 'key' AND p.source <> 'accounting-proxy' THEN p.figure END, \
            price_source = 'catalog' \
           FROM priced p WHERE a.id = p.id",
    )
    .bind(scope.environment_id())
    .bind(model)
    .bind(price.input_per_mtok)
    .bind(price.output_per_mtok)
    .bind(price.cached_rate())
    .execute(scope.connection())
    .await?;
    Ok(result.rows_affected())
}
