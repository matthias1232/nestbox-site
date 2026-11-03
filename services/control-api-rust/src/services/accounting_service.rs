//! Request accounting: what a request used, what it cost, and what a window
//! shows.
//!
//! # Three decisions worth knowing
//!
//! 1. **A request is recorded with the price that was known at that moment.**
//!    The cost is derived in [`record`] and stored next to the tokens, so a later
//!    price change does not rewrite history. [`price_for`] is the same resolution
//!    the write path uses, which is what makes a stored cost reproducible by
//!    hand.
//! 2. **No price is `None`, not zero.** [`price_for`] answers `None` when neither
//!    the environment's price list nor the global fallback names the model, and
//!    the row is written with `cost_micros = NULL` and `price_source =
//!    unknown`. The overview reports `price_missing_requests` next to the totals,
//!    so a partial total cannot be mistaken for a complete one.
//! 3. **The proxy is a tenant, not a principal.** [`record`] takes the
//!    environment from the row (it is the proxy's own identity, the same
//!    inversion `telemetry_service` uses for the ingest batch) and still opens
//!    the tenant scope, checks the sandbox belongs to that environment, and
//!    writes through the store's scoped connection. Every read path takes a
//!    `&Principal` and derives the environment from it.
//!
//! # Price resolution
//!
//! A sandbox's provider profile names the provider and model; it carries no
//! prices (`provider_profiles` has no price columns, by design — a price is
//! billable data, not a credential or a connection). Resolution is therefore:
//! the environment's `model_prices` row, then the global row
//! (`environment_id IS NULL`), then nothing. [`PriceSource::Provider`] exists for
//! a cost the provider itself reports on the request; [`record`] stores that when
//! it is present instead of deriving one.
//!
//! # Permissions
//!
//! Reading is `TelemetryRead`: what a sandbox used is telemetry, and an operator
//! who may see its traffic may see its tokens. Writing a price, changing the
//! retention and purging are `PolicyGlobal`, because each one changes a number
//! every sandbox in the environment is billed against, and the purge decides when
//! evidence disappears.
//!
//! # Retention
//!
//! Raw rows are worth their disk for a window, not forever. The setting is one
//! row per environment, [`purge`] folds the rows into `accounting_daily` first
//! when `keep_daily` is on, and only then deletes raw rows older than the cutoff.
//! The rollup is never purged here: losing it would turn a retention sweep into a
//! history loss.

use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::json;
use utoipa::ToSchema;
use uuid::Uuid;

use super::ServiceContext;
use super::access_control::require;
use crate::domain::accounting::{
    AccountingRow, AccountingSettings, MAX_MODEL_NAME, MAX_RETENTION_DAYS, MIN_RETENTION_DAYS,
    NewAccountingRow, Overview, Price, PriceSource, Timeframe, Usage, cost_micros,
};
use crate::domain::auth::{Permission, Principal};
use crate::error::{ApiError, ErrorCode};
use crate::store::TenantScope;
use crate::store::postgres::accounting as store;

/// The largest page of request rows a caller may ask for.
///
/// A window with a million requests has a summary the console can draw and a list
/// it cannot; the page bound is the difference between a screen and a download.
pub const MAX_REQUEST_PAGE: i64 = 500;

/// What [`record`] stored, so the proxy can log or return the same cost the row
/// holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
pub struct RecordedRequest {
    /// The row's id.
    pub id: Uuid,
    /// `None` when no price was known; never zero as a stand-in.
    pub cost_micros: Option<i64>,
    /// Where the cost came from.
    pub price_source: PriceSource,
}

/// What a purge did, so an operator sees more than a count.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
pub struct PurgeOutcome {
    /// How many raw request rows were removed.
    pub removed_requests: u64,
    /// How many daily buckets were (re)written before the delete.
    pub rolled_up_rows: u64,
    /// The retention the sweep applied.
    pub retention_days: i32,
    /// Whether the daily rollup was kept.
    pub keep_daily: bool,
    /// The moment the raw table was cut at.
    pub cutoff: DateTime<Utc>,
}

/// Resolve the price of one model for the scope's environment.
///
/// The order is the environment's row, then the global row, then nothing. The
/// sandbox step in front of it is its environment: a sandbox has no price list of
/// its own, and the provider profile it uses carries no prices — so there is no
/// fourth source to fall back to, and `None` is the honest answer rather than a
/// guessed rate.
///
/// The scope is the parameter rather than an id because the caller already holds
/// one: during [`record`] the price lookup and the insert must see the same
/// transaction, and a second scope would be a second snapshot.
pub async fn price_for(
    scope: &mut TenantScope<'_>,
    model: &str,
) -> Result<Option<(Price, PriceSource)>, ApiError> {
    let model = model.trim();
    if model.is_empty() {
        return Ok(None);
    }
    store::price_for_model(scope, scope.environment_id(), model)
        .await
        .map_err(ApiError::from)
}

/// Record one request for a sandbox and return its stored id and cost.
///
/// This is the proxy's write path, so it takes the environment from the row and
/// opens the tenant scope itself. It still refuses a sandbox that is not in that
/// environment: an accounting row in the wrong tenant is as bad as a traffic row
/// in the wrong timeline, and a silent write is worse than a 404 the proxy can
/// log.
///
/// There is no permission check because the caller is not a user: the proxy
/// authenticates as a node, and a node's authority is checked where the request
/// enters (the internal ingest route resolves the node token). A principal check
/// here would be a check against a principal that does not exist.
pub async fn record(
    ctx: &ServiceContext,
    mut request: NewAccountingRow,
) -> Result<RecordedRequest, ApiError> {
    // Normalise in place before anything is read: a blank string is `NULL`, a
    // negative count is zero, and a body that was sent by mistake is not a
    // digest. Doing it here means the row that is priced is the row that is
    // stored.
    request.usage = Usage::new(
        request.usage.prompt,
        request.usage.completion,
        request.usage.cached,
        request.usage.reasoning,
    );
    request.content_digest = digest(request.content_digest.as_deref()).map(str::to_owned);
    request.agent_instance = text(request.agent_instance.as_deref()).map(str::to_owned);
    request.provider = text(request.provider.as_deref()).map(str::to_owned);
    request.model = text(request.model.as_deref()).map(str::to_owned);
    request.endpoint = text(request.endpoint.as_deref()).map(str::to_owned);
    request.privacy_action = text(request.privacy_action.as_deref()).map(str::to_owned);
    request.privacy_detector = text(request.privacy_detector.as_deref()).map(str::to_owned);
    request.status = request.status.max(0);
    request.latency_ms = request.latency_ms.max(0);

    let mut scope = TenantScope::begin(&ctx.pool, request.environment_id)
        .await
        .map_err(ApiError::from)?;
    if !store::sandbox_exists(&mut scope, request.sandbox_id)
        .await
        .map_err(ApiError::from)?
    {
        return Err(ApiError::not_found(
            ErrorCode::SANDBOX_NOT_FOUND,
            "No such sandbox in this environment.",
        ));
    }

    let (cost, price_source) = match request.reported_cost_micros {
        Some(reported) => (Some(reported.max(0)), PriceSource::Provider),
        None => match request.model.as_deref() {
            Some(model) => match price_for(&mut scope, model).await? {
                Some((price, source)) => (cost_micros(&request.usage, Some(&price)), source),
                None => (None, PriceSource::Unknown),
            },
            None => (None, PriceSource::Unknown),
        },
    };

    let id = store::insert_request(&mut scope, &request, cost, price_source)
        .await
        .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;

    Ok(RecordedRequest {
        id,
        cost_micros: cost,
        price_source,
    })
}

/// The accounting window: totals, per model, per sandbox, and the privacy
/// verdicts of the same interval.
///
/// `sandbox_id: Some(_)` narrows every number to one sandbox and leaves
/// `sandboxes` empty, because a one-row breakdown is noise. The sandbox is
/// checked to be in the caller's environment before it is queried: an unknown id
/// is a 404, not an empty window, because the two mean different things to the
/// person reading the screen.
pub async fn overview(
    ctx: &ServiceContext,
    principal: &Principal,
    timeframe: Timeframe,
    sandbox_id: Option<Uuid>,
) -> Result<Overview, ApiError> {
    require(principal, Permission::TelemetryRead)?;
    let since = timeframe.since(Utc::now());

    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    ensure_sandbox(&mut scope, sandbox_id).await?;

    let totals = store::totals(&mut scope, sandbox_id, since)
        .await
        .map_err(ApiError::from)?;
    let summaries = store::summary_by_model(&mut scope, sandbox_id, since)
        .await
        .map_err(ApiError::from)?;
    let sandboxes = if sandbox_id.is_none() {
        store::summary_by_sandbox(&mut scope, since)
            .await
            .map_err(ApiError::from)?
    } else {
        Vec::new()
    };
    let privacy = store::privacy_stats(&mut scope, sandbox_id, since)
        .await
        .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;

    Ok(Overview {
        timeframe,
        since,
        summaries,
        sandboxes,
        privacy,
        totals,
        price_missing_requests: totals.price_missing_requests,
    })
}

/// The newest request rows of an environment or one sandbox.
///
/// A separate read from [`overview`] on purpose: a window with a million requests
/// has a summary the console can draw and a list it cannot. The bound is the
/// caller's page size, clamped, and the window start comes from the timeframe so
/// the list and the summary describe the same interval.
pub async fn requests(
    ctx: &ServiceContext,
    principal: &Principal,
    timeframe: Timeframe,
    sandbox_id: Option<Uuid>,
    limit: i64,
) -> Result<Vec<AccountingRow>, ApiError> {
    require(principal, Permission::TelemetryRead)?;
    let limit = if limit <= 0 {
        100
    } else {
        limit.min(MAX_REQUEST_PAGE)
    };
    let since = timeframe.since(Utc::now());

    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    ensure_sandbox(&mut scope, sandbox_id).await?;
    let rows = store::list_requests(&mut scope, sandbox_id, since, limit)
        .await
        .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(rows)
}

/// Set the price of one model for the caller's environment.
///
/// `PolicyGlobal` because the price is what every sandbox in the environment is
/// billed against: a per-user rate would make the overview depend on who opened
/// it. The model name is trimmed and bounded, and every rate is checked before it
/// reaches the column's CHECK, so a bad number is a 422 with a sentence rather
/// than a constraint name.
///
/// The write is an upsert on `(environment_id, model)`: two saves of the same
/// model are the same row, and the later one wins. That is the honest behaviour
/// for a price list, where the newest decision is the current one.
pub async fn set_price(
    ctx: &ServiceContext,
    principal: &Principal,
    model: &str,
    price: Price,
) -> Result<Price, ApiError> {
    require(principal, Permission::PolicyGlobal)?;
    let model = model.trim();
    if model.is_empty() || model.chars().count() > MAX_MODEL_NAME {
        return Err(ApiError::validation(format!(
            "model must be between 1 and {MAX_MODEL_NAME} characters."
        )));
    }
    if let Some(reason) = price.invalid_reason() {
        return Err(ApiError::validation(reason));
    }

    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    store::upsert_price(&mut scope, Some(principal.environment_id), model, &price)
        .await
        .map_err(ApiError::from)?;
    // The calls already stored without a price get one now, so the page does not
    // show n/a for last week's traffic of a model that has a price today.
    let repriced = store::reprice_model(&mut scope, model, &price)
        .await
        .map_err(ApiError::from)?;
    tracing::info!(model, repriced, "price set, earlier calls priced");
    crate::store::postgres::audit::record(
        &mut scope,
        crate::store::postgres::audit::NewAuditEvent {
            actor_id: Some(principal.user_id),
            actor_label: &principal.display_name,
            kind: "accounting.price",
            subject_type: Some("environment"),
            subject_id: Some(principal.environment_id),
            detail: &json!({
                "model": model,
                "input_micros_per_mtok": price.input_per_mtok,
                "output_micros_per_mtok": price.output_per_mtok,
                "cached_micros_per_mtok": price.cached_per_mtok,
            }),
            request_id: None,
        },
    )
    .await
    .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(price)
}

/// The environment's retention setting, or the default when it never saved one.
///
/// `TelemetryRead`, not an admin permission: knowing how long the numbers are
/// kept is part of reading them, and a default is not a secret.
pub async fn settings(
    ctx: &ServiceContext,
    principal: &Principal,
) -> Result<AccountingSettings, ApiError> {
    require(principal, Permission::TelemetryRead)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let settings = store::get_settings(&mut scope)
        .await
        .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(settings)
}

/// Change how long raw rows are kept and whether the rollup outlives them.
///
/// `PolicyGlobal` because it is one policy for every sandbox in the environment
/// and it decides when evidence disappears. The save is the whole setting, so a
/// request that changes the window must name `keep_daily` too — a patch would
/// leave the other half decided by whatever the row held a moment earlier.
pub async fn set_settings(
    ctx: &ServiceContext,
    principal: &Principal,
    update: AccountingSettings,
) -> Result<AccountingSettings, ApiError> {
    require(principal, Permission::PolicyGlobal)?;
    if let Some(reason) = update.invalid_reason() {
        return Err(ApiError::validation(reason));
    }

    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let saved = AccountingSettings {
        retention_days: update.retention_days,
        keep_daily: update.keep_daily,
        updated_by: Some(principal.user_id),
        updated_at: None,
    };
    store::save_settings(&mut scope, &saved, principal.user_id)
        .await
        .map_err(ApiError::from)?;
    crate::store::postgres::audit::record(
        &mut scope,
        crate::store::postgres::audit::NewAuditEvent {
            actor_id: Some(principal.user_id),
            actor_label: &principal.display_name,
            kind: "accounting.settings",
            subject_type: Some("environment"),
            subject_id: Some(principal.environment_id),
            detail: &json!({
                "retention_days": saved.retention_days,
                "keep_daily": saved.keep_daily,
            }),
            request_id: None,
        },
    )
    .await
    .map_err(ApiError::from)?;
    let stored = store::get_settings(&mut scope)
        .await
        .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(stored)
}

/// Apply the environment's retention policy: roll up, then delete raw rows.
///
/// `PolicyGlobal` for the same reason as [`set_settings`], and because this is
/// the destructive half. The setting decides the window, so a scheduled sweep and
/// a click in the console apply the same policy.
pub async fn purge(ctx: &ServiceContext, principal: &Principal) -> Result<PurgeOutcome, ApiError> {
    require(principal, Permission::PolicyGlobal)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let settings = store::get_settings(&mut scope)
        .await
        .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    run_purge(ctx, principal, settings.retention_days, settings.keep_daily).await
}

/// Purge with an explicit window, for the console's "delete rows older than N
/// days" action.
///
/// Separate from [`purge`] rather than an optional argument: the two decide
/// different things. [`purge`] applies the saved policy and is what a schedule
/// calls; this one is a one-off that must not silently rewrite the setting, so
/// the retention it applied is reported back instead of stored. `keep_daily`
/// still comes from the setting, because the rollup decision is a policy and not
/// a parameter of one sweep.
pub async fn purge_older_than(
    ctx: &ServiceContext,
    principal: &Principal,
    retention_days: i32,
) -> Result<PurgeOutcome, ApiError> {
    require(principal, Permission::PolicyGlobal)?;
    if !(MIN_RETENTION_DAYS..=MAX_RETENTION_DAYS).contains(&retention_days) {
        return Err(ApiError::validation(format!(
            "retention_days must be between {MIN_RETENTION_DAYS} and {MAX_RETENTION_DAYS}."
        )));
    }
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let settings = store::get_settings(&mut scope)
        .await
        .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    run_purge(ctx, principal, retention_days, settings.keep_daily).await
}

/// The shared body of both purge paths.
///
/// The rollup runs inside the same transaction as the delete, so a crash between
/// them cannot leave a window that is neither in the raw table nor in the daily
/// one. `keep_daily = false` is honoured: an environment that wants no long-term
/// history gets exactly that, and the audit row records which policy ran.
async fn run_purge(
    ctx: &ServiceContext,
    principal: &Principal,
    retention_days: i32,
    keep_daily: bool,
) -> Result<PurgeOutcome, ApiError> {
    // The cutoff comes from the domain type so the sweep and `settings.cutoff`
    // cannot disagree about what "30 days" means.
    let policy = AccountingSettings {
        retention_days,
        keep_daily,
        updated_by: None,
        updated_at: None,
    };
    let cutoff = policy.cutoff(Utc::now());
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let rolled_up_rows = if keep_daily {
        // The whole raw table, not just the window after the cutoff: the rows
        // the cutoff removes are exactly the ones that must already be in the
        // rollup, and a window that started at the cutoff would aggregate the
        // survivors and then delete the history it skipped. The upsert is
        // idempotent, so the buckets inside the retention window are simply
        // recomputed.
        store::rollup_daily(&mut scope, DateTime::UNIX_EPOCH)
            .await
            .map_err(ApiError::from)?
    } else {
        0
    };
    let removed_requests = store::purge_before(&mut scope, cutoff)
        .await
        .map_err(ApiError::from)?;
    crate::store::postgres::audit::record(
        &mut scope,
        crate::store::postgres::audit::NewAuditEvent {
            actor_id: Some(principal.user_id),
            actor_label: &principal.display_name,
            kind: "accounting.purge",
            subject_type: Some("environment"),
            subject_id: Some(principal.environment_id),
            detail: &json!({
                "retention_days": retention_days,
                "keep_daily": keep_daily,
                "cutoff": cutoff.to_rfc3339(),
                "removed_requests": removed_requests,
                "rolled_up_rows": rolled_up_rows,
            }),
            request_id: None,
        },
    )
    .await
    .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;

    Ok(PurgeOutcome {
        removed_requests,
        rolled_up_rows,
        retention_days,
        keep_daily,
        cutoff,
    })
}

/// Refuse a sandbox that is not in the scope's environment.
///
/// Shared by the two read paths so neither can forget the check: a filter that
/// matches nothing and a sandbox that does not exist render as the same empty
/// table, and only one of them is a bug the caller should fix.
async fn ensure_sandbox(
    scope: &mut TenantScope<'_>,
    sandbox_id: Option<Uuid>,
) -> Result<(), ApiError> {
    let Some(sandbox_id) = sandbox_id else {
        return Ok(());
    };
    if store::sandbox_exists(scope, sandbox_id)
        .await
        .map_err(ApiError::from)?
    {
        return Ok(());
    }
    Err(ApiError::not_found(
        ErrorCode::SANDBOX_NOT_FOUND,
        "No such sandbox in this environment.",
    ))
}

/// Drop a blank optional text, so an unset field is `NULL` and not `""`.
///
/// The proxy reads its identity from environment variables that may be empty.
/// Storing the empty string would make "the proxy did not know" look like a
/// provider called `""` in every group-by.
fn text(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

/// Accept a value as a content digest only when it is a SHA-256 hex string.
///
/// The column is bounded, but a bound is not a guarantee of content-freedom.
/// Requiring the exact shape means a caller that mistakenly sent the request body
/// gets `NULL` stored, not the first 128 bytes of it.
fn digest(value: Option<&str>) -> Option<&str> {
    let value = value?.trim();
    let is_hex = value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit());
    if is_hex { Some(value) } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::accounting::DEFAULT_RETENTION_DAYS;

    #[test]
    fn a_digest_is_stored_only_when_it_is_a_sha256() {
        let hash = "a".repeat(64);
        assert_eq!(digest(Some(&hash)), Some(hash.as_str()));

        // The body itself must not become a stored excerpt.
        assert_eq!(digest(Some("the customer's prompt, in full")), None);
        assert_eq!(digest(Some("sha256:not-a-hash")), None);
        assert_eq!(digest(Some(&"a".repeat(63))), None);
        assert_eq!(digest(None), None);
    }

    #[test]
    fn a_blank_optional_text_is_null() {
        assert_eq!(text(Some("  ")), None);
        assert_eq!(text(Some("")), None);
        assert_eq!(text(None), None);
        assert_eq!(text(Some(" anthropic ")), Some("anthropic"));
    }

    #[test]
    fn the_retention_bounds_are_the_documented_ones() {
        assert_eq!(MIN_RETENTION_DAYS, 1);
        assert_eq!(DEFAULT_RETENTION_DAYS, 30);
        assert_eq!(MAX_RETENTION_DAYS, 3_650);
        assert!(AccountingSettings::default().invalid_reason().is_none());
    }
}


// ---------------------------------------------------------------------------
// Usage read off the transparent proxy
// ---------------------------------------------------------------------------

/// One model call as the egress proxy saw it.
pub struct ObservedCall {
    /// The traffic event the counters were read from (idempotency key).
    pub id: Uuid,
    pub environment_id: Uuid,
    pub sandbox_id: Uuid,
    pub host: String,
    pub path: Option<String>,
    pub status: i32,
    pub latency_ms: i32,
    pub model: Option<String>,
    pub usage: crate::domain::accounting::ObservedUsage,
    pub cache_markers: i32,
    pub auth: crate::domain::accounting::AuthKind,
    pub conversation: Option<String>,
}

/// The provider a host belongs to, by name. Unknown hosts keep their own name, so
/// a self-hosted gateway still appears as itself instead of vanishing.
pub fn provider_of(host: &str) -> String {
    let host = host.trim().to_ascii_lowercase();
    let host = host.split(':').next().unwrap_or("");
    match host {
        "api.anthropic.com" => "anthropic".into(),
        "api.openai.com" => "openai".into(),
        "chatgpt.com" | "chat.openai.com" => "codex".into(),
        "generativelanguage.googleapis.com" => "google".into(),
        "openrouter.ai" => "openrouter".into(),
        "api.mistral.ai" => "mistral".into(),
        "api.groq.com" => "groq".into(),
        "api.x.ai" => "xai".into(),
        "api.deepseek.com" => "deepseek".into(),
        other => other.to_string(),
    }
}

/// Store a call the egress proxy read, priced.
///
/// Only a call made with an API key gets a `cost_micros`. A call made through a
/// signed-in account has no per-token price (the subscription is the price), so
/// its list-price figure is an *estimate*, kept in its own column and shown on its
/// own line. A call with no known price has neither, and is counted as unpriced.
///
/// Best effort for the caller: the telemetry batch has already stored the traffic
/// row, and a failure here must not turn into a refused batch.
pub async fn record_observed(ctx: &ServiceContext, call: ObservedCall) -> Result<(), ApiError> {
    use crate::domain::accounting::{AuthKind, observed_cost_micros};

    let usage = call.usage.normalised();
    if usage.prompt() + usage.output <= 0 {
        return Ok(());
    }
    let mut scope = TenantScope::begin(&ctx.pool, call.environment_id)
        .await
        .map_err(ApiError::from)?;
    if !store::sandbox_exists(&mut scope, call.sandbox_id)
        .await
        .map_err(ApiError::from)?
    {
        return Ok(());
    }
    let price = match call.model.as_deref() {
        Some(model) => price_for(&mut scope, model).await?,
        None => None,
    };
    let (cost, estimate, source) = match &price {
        Some((price, source)) => {
            let figure = observed_cost_micros(&usage, Some(price));
            match call.auth {
                AuthKind::Key => (figure, None, *source),
                _ => (None, figure, *source),
            }
        }
        None => (None, None, PriceSource::Unknown),
    };
    let provider = provider_of(&call.host);
    store::insert_observed(
        &mut scope,
        &store::ObservedRow {
            id: call.id,
            sandbox_id: call.sandbox_id,
            provider: &provider,
            model: call.model.as_deref(),
            endpoint: call.path.as_deref().map(|path| path.split('?').next().unwrap_or(path)),
            status: call.status,
            latency_ms: call.latency_ms,
            usage,
            cache_markers: call.cache_markers,
            auth: call.auth,
            conversation: call.conversation.as_deref(),
            cost_micros: cost,
            estimated_cost_micros: estimate,
            price_source: source,
        },
    )
    .await
    .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(())
}

/// The answer to "how much, of what, by whom".
#[derive(Debug, Serialize, ToSchema)]
pub struct UsageReport {
    pub group_by: String,
    pub since: chrono::DateTime<Utc>,
    pub rows: Vec<store::UsageRow>,
    pub totals: store::UsageRow,
}

/// Usage over a window, grouped by one dimension.
///
/// An admin sees every sandbox of the environment; anyone else sees their own.
/// `sandbox_id` narrows to one sandbox and is checked to exist first.
pub async fn usage(
    ctx: &ServiceContext,
    principal: &Principal,
    timeframe: Timeframe,
    group_by: &str,
    sandbox_id: Option<Uuid>,
) -> Result<UsageReport, ApiError> {
    require(principal, Permission::TelemetryRead)?;
    let group = store::UsageGroup::parse(group_by).ok_or_else(|| {
        ApiError::validation(
            "group_by is provider, model, sandbox, auth, day, owner, site, organization, agent or conversation.",
        )
    })?;
    let since = timeframe.since(Utc::now());
    let owner = if principal.role == crate::domain::auth::Role::Admin {
        None
    } else {
        Some(principal.user_id)
    };

    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    ensure_sandbox(&mut scope, sandbox_id).await?;
    let rows = store::usage_breakdown(&mut scope, group, sandbox_id, owner, since)
        .await
        .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;

    let mut totals = store::UsageRow {
        key: "total".into(),
        requests: 0,
        prompt_tokens: 0,
        completion_tokens: 0,
        cache_read_tokens: 0,
        cache_write_tokens: 0,
        cache_write_1h_tokens: 0,
        reasoning_tokens: 0,
        cache_markers: 0,
        cost_micros: None,
        estimated_cost_micros: None,
        unpriced_requests: 0,
        account_requests: 0,
        key_requests: 0,
        cache_hit_rate: 0.0,
    };
    for row in &rows {
        totals.requests += row.requests;
        totals.prompt_tokens += row.prompt_tokens;
        totals.completion_tokens += row.completion_tokens;
        totals.cache_read_tokens += row.cache_read_tokens;
        totals.cache_write_tokens += row.cache_write_tokens;
        totals.cache_write_1h_tokens += row.cache_write_1h_tokens;
        totals.reasoning_tokens += row.reasoning_tokens;
        totals.cache_markers += row.cache_markers;
        totals.unpriced_requests += row.unpriced_requests;
        totals.account_requests += row.account_requests;
        totals.key_requests += row.key_requests;
        totals.cost_micros = sum_option(totals.cost_micros, row.cost_micros);
        totals.estimated_cost_micros = sum_option(totals.estimated_cost_micros, row.estimated_cost_micros);
    }
    if totals.prompt_tokens > 0 {
        totals.cache_hit_rate = totals.cache_read_tokens as f64 / totals.prompt_tokens as f64;
    }
    Ok(UsageReport { group_by: group_by.to_string(), since, rows, totals })
}

/// Sum that stays `None` while nothing contributed, so "no price" is not "zero".
fn sum_option(a: Option<i64>, b: Option<i64>) -> Option<i64> {
    match (a, b) {
        (None, None) => None,
        (a, b) => Some(a.unwrap_or(0).saturating_add(b.unwrap_or(0))),
    }
}

#[cfg(test)]
mod observed_service_tests {
    use super::*;

    #[test]
    fn hosts_map_to_providers_and_unknown_hosts_keep_their_name() {
        assert_eq!(provider_of("api.anthropic.com"), "anthropic");
        assert_eq!(provider_of("API.OpenAI.com:443"), "openai");
        assert_eq!(provider_of("chatgpt.com"), "codex");
        assert_eq!(provider_of("llm.internal.example"), "llm.internal.example");
    }

    #[test]
    fn a_sum_with_nothing_in_it_is_not_zero() {
        assert_eq!(sum_option(None, None), None);
        assert_eq!(sum_option(None, Some(5)), Some(5));
        assert_eq!(sum_option(Some(2), Some(3)), Some(5));
    }
}


/// The price list and the models that still lack a price.
#[derive(Debug, Serialize, ToSchema)]
pub struct PriceList {
    pub prices: Vec<store::PriceEntry>,
    pub unpriced: Vec<UnpricedModel>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct UnpricedModel {
    pub model: String,
    pub calls: i64,
    pub tokens: i64,
}

/// What prices exist and which used models have none (last 30 days).
pub async fn prices(ctx: &ServiceContext, principal: &Principal) -> Result<PriceList, ApiError> {
    require(principal, Permission::TelemetryRead)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let prices = store::list_prices(&mut scope).await.map_err(ApiError::from)?;
    let unpriced = store::unpriced_models(&mut scope, Utc::now() - chrono::Duration::days(30))
        .await
        .map_err(ApiError::from)?
        .into_iter()
        .map(|(model, calls, tokens)| UnpricedModel { model, calls, tokens })
        .collect();
    scope.commit().await.map_err(ApiError::from)?;
    Ok(PriceList { prices, unpriced })
}
