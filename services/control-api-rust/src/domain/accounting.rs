//! Accounting: what a request used, what it cost, and what a window shows.
//!
//! The rule this module exists for: **a cost is either known or it is `None`.**
//! There is no zero as a stand-in, because zero is indistinguishable from "free"
//! and a total assembled from free-looking rows is a number that cannot be
//! defended. Every type here says that with an `Option`.
//!
//! Everything is pure integer arithmetic in micros (one millionth of a currency
//! unit). Floating point would make the price stored in `model_prices` and the
//! cost written next to a request disagree in the last digit, and the digit an
//! invoice is compared against is the one that must not drift.
//!
//! The billing split: `cached` is a subset of `prompt`, so billable input is
//! `prompt - cached`; `reasoning` is already inside `completion` for every
//! provider that reports it separately, so it is recorded but never billed
//! twice.
//!
//! This layer performs no I/O. The arithmetic is here so it can be tested
//! without a database, and so the service and the rollup cannot disagree about
//! what a token costs.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// The retention an environment gets before it saves a setting.
///
/// Kept here so the service default and the column default in
/// `0058_accounting.sql` cannot drift apart.
pub const DEFAULT_RETENTION_DAYS: i32 = 30;

/// The smallest retention a setting may name.
pub const MIN_RETENTION_DAYS: i32 = 1;

/// The largest retention a setting may name, matching the column's CHECK.
pub const MAX_RETENTION_DAYS: i32 = 3_650;

/// The longest model name a price row may carry, matching the column's CHECK.
pub const MAX_MODEL_NAME: usize = 200;

/// How many tokens a per-million rate is quoted for.
const TOKENS_PER_MTOK: i128 = 1_000_000;

/// The tokens one request used, as the provider reported them.
///
/// Constructed with [`Usage::new`] when the values come from a caller, so a
/// negative count or a cached count larger than the prompt cannot reach the
/// database's CHECK constraints as a 500.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, ToSchema)]
pub struct Usage {
    /// All prompt tokens, cached ones included.
    pub prompt: i64,
    /// All completion tokens, reasoning ones included.
    pub completion: i64,
    /// The part of `prompt` the provider served from its cache.
    pub cached: i64,
    /// The part of `completion` the provider spent reasoning.
    pub reasoning: i64,
}

impl Usage {
    /// Build a usage record, clamping it into the shape the schema accepts.
    ///
    /// A provider that reports a cached count above its prompt count is telling
    /// us something inconsistent; clamping is the honest repair, and the result
    /// is still a number the cost arithmetic can use.
    pub fn new(prompt: i64, completion: i64, cached: i64, reasoning: i64) -> Self {
        let prompt = prompt.max(0);
        Self {
            prompt,
            completion: completion.max(0),
            cached: cached.clamp(0, prompt),
            reasoning: reasoning.max(0),
        }
    }

    /// The prompt tokens billed at the full input rate.
    ///
    /// `cached` is a subset of `prompt`, so charging the whole prompt at the
    /// input rate and the cached part again would bill those tokens twice.
    pub fn billable_prompt(&self) -> i64 {
        self.prompt.saturating_sub(self.cached)
    }
}

/// A price list entry: micros per million tokens.
///
/// `cached_per_mtok` is optional because a provider that discounts cached tokens
/// may simply not have said so; [`Price::cached_rate`] then charges them as
/// ordinary input rather than inventing a discount.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct Price {
    /// Micros charged per million prompt tokens.
    pub input_per_mtok: i64,
    /// Micros charged per million completion tokens.
    pub output_per_mtok: i64,
    /// Micros charged per million cached prompt tokens, when the provider says.
    pub cached_per_mtok: Option<i64>,
}

impl Price {
    /// The rate cached prompt tokens are billed at.
    ///
    /// No separate cached rate means the provider charges them like input; that
    /// is a real answer, not a missing one.
    pub fn cached_rate(&self) -> i64 {
        self.cached_per_mtok.unwrap_or(self.input_per_mtok)
    }

    /// Whether every rate is non-negative and the model could be stored.
    ///
    /// Returns the sentence a caller can act on, or `None` when the value is
    /// storable. It is here rather than in the service so the same rule covers
    /// any future writer of `model_prices`.
    pub fn invalid_reason(&self) -> Option<&'static str> {
        if self.input_per_mtok < 0 {
            return Some("input_per_mtok must not be negative.");
        }
        if self.output_per_mtok < 0 {
            return Some("output_per_mtok must not be negative.");
        }
        if self.cached_per_mtok.is_some_and(|rate| rate < 0) {
            return Some("cached_per_mtok must not be negative.");
        }
        None
    }
}

/// What one request cost, in micros, or `None` when no price was known.
///
/// The split is `billable_prompt` at the input rate, `cached` at the cached
/// rate, and the whole `completion` at the output rate. Reasoning tokens are
/// part of `completion` and are therefore not added again.
///
/// Rounding is half up to the nearest micro, so a sub-micro request costs one
/// micro rather than disappearing: a million free-looking rows are a bug, not a
/// discount. `None` in means `None` out — an unpriced request has no cost, and
/// zero would read as "free".
pub fn cost_micros(usage: &Usage, price: Option<&Price>) -> Option<i64> {
    let price = price?;
    let prompt = usage.prompt.max(0);
    let cached = usage.cached.clamp(0, prompt);
    let billable_prompt = prompt - cached;
    let completion = usage.completion.max(0);

    let micros = (billable_prompt as i128) * (price.input_per_mtok.max(0) as i128)
        + (cached as i128) * (price.cached_rate().max(0) as i128)
        + (completion as i128) * (price.output_per_mtok.max(0) as i128);

    Some(round_to_micros(micros))
}

/// Divide a token-times-rate product down to micros, rounding half up.
///
/// Saturates rather than wrapping: a cost that overflows `i64` is absurd, but a
/// wrapped negative total would be worse than an obviously capped one.
fn round_to_micros(numerator: i128) -> i64 {
    let rounded = (numerator + TOKENS_PER_MTOK / 2) / TOKENS_PER_MTOK;
    rounded.clamp(0, i64::MAX as i128) as i64
}

/// Where a `cost_micros` value came from.
///
/// `Unknown` is not an error: it is the honest label of a request whose model had
/// no price, and it is what lets a console show coverage instead of a total that
/// silently excludes half the traffic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PriceSource {
    /// A row in `model_prices`, environment-scoped or global.
    Catalog,
    /// A cost the provider itself reported for the request.
    Provider,
    /// No price was known; `cost_micros` is `None`.
    #[default]
    Unknown,
}

impl PriceSource {
    /// The database and wire spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Catalog => "catalog",
            Self::Provider => "provider",
            Self::Unknown => "unknown",
        }
    }

    /// Read the database spelling; unknown input is `None`, never a default.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "catalog" => Some(Self::Catalog),
            "provider" => Some(Self::Provider),
            "unknown" => Some(Self::Unknown),
            _ => None,
        }
    }
}

/// Which way a request travelled, as the accounting proxy records it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    /// A request arriving at the sandbox.
    #[serde(alias = "inbound")]
    In,
    /// A request leaving the sandbox for a provider.
    #[serde(alias = "outbound")]
    Out,
}

impl Direction {
    /// The database and wire spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::In => "in",
            Self::Out => "out",
        }
    }

    /// Read the database or wire spelling; unknown input is `None`.
    ///
    /// Both spellings are accepted: the accounting proxy speaks `outbound` /
    /// `inbound` (the words the privacy layer uses), and the table stores the
    /// short `out` / `in`. Normalising here means a caller cannot write a row
    /// the column's CHECK would refuse, and a reader never sees two spellings of
    /// one direction.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "in" | "inbound" => Some(Self::In),
            "out" | "outbound" => Some(Self::Out),
            _ => None,
        }
    }
}

/// The windows the overview offers.
///
/// A closed set, so the console has five buttons and the server has five bounds.
/// The wire spelling is the human one (`5m`, `24h`), because it is what a link
/// carries and what an operator types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, ToSchema)]
pub enum Timeframe {
    /// The last five minutes.
    #[serde(rename = "5m")]
    M5,
    /// The last hour.
    #[serde(rename = "1h")]
    H1,
    /// The last twenty-four hours.
    #[serde(rename = "24h")]
    #[default]
    H24,
    /// The last seven days.
    #[serde(rename = "7d")]
    D7,
    /// The last thirty days.
    #[serde(rename = "30d")]
    D30,
}

impl Timeframe {
    /// The link and wire spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::M5 => "5m",
            Self::H1 => "1h",
            Self::H24 => "24h",
            Self::D7 => "7d",
            Self::D30 => "30d",
        }
    }

    /// Read the wire spelling; unknown input is `None` rather than a guess.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "5m" => Some(Self::M5),
            "1h" => Some(Self::H1),
            "24h" => Some(Self::H24),
            "7d" => Some(Self::D7),
            "30d" => Some(Self::D30),
            _ => None,
        }
    }

    /// The window's length in seconds.
    ///
    /// A month is thirty days here, not a calendar month: the bound is applied to
    /// `created_at`, and a length that varied by month would make two clicks on
    /// "30d" describe two different intervals.
    pub fn seconds(self) -> i64 {
        match self {
            Self::M5 => 300,
            Self::H1 => 3_600,
            Self::H24 => 86_400,
            Self::D7 => 604_800,
            Self::D30 => 2_592_000,
        }
    }

    /// The moment this window starts, relative to `now`.
    ///
    /// One function so a query bound is never computed from a different clock
    /// than the one that answered the request.
    pub fn since(self, now: DateTime<Utc>) -> DateTime<Utc> {
        now - Duration::seconds(self.seconds())
    }
}

/// One request as the accounting proxy reports it, before it is priced.
///
/// This is the ingest shape, so it carries what the proxy knows and nothing it
/// does not: no prompt, no completion, no derived cost. `cost_micros` is filled
/// by the service from the price list, while `reported_cost_micros` is the
/// provider's own figure when it sent one — the two are kept apart so a derived
/// cost can never be mistaken for a billed one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct NewAccountingRow {
    /// The tenant the proxy is writing for.
    pub environment_id: Uuid,
    /// The sandbox the request belongs to; must be in `environment_id`.
    pub sandbox_id: Uuid,
    /// Which agent inside the sandbox made it, when the proxy knows.
    pub agent_instance: Option<String>,
    /// The provider the request went to (or came from).
    pub provider: Option<String>,
    /// The model the provider named.
    pub model: Option<String>,
    /// Which way the request travelled.
    pub direction: Direction,
    /// The path the proxy saw.
    pub endpoint: Option<String>,
    /// The HTTP status the proxy returned.
    pub status: i32,
    /// How long the request took, in milliseconds.
    pub latency_ms: i32,
    /// The tokens the provider reported; zeroes when it reported none.
    pub usage: Usage,
    /// SHA-256 of the content, lowercase hex. Never the content itself.
    pub content_digest: Option<String>,
    /// The cost the provider reported, when it reported one.
    pub reported_cost_micros: Option<i64>,
    /// What the privacy layer did, when it ran.
    pub privacy_action: Option<String>,
    /// Which detector decided it, when one did.
    pub privacy_detector: Option<String>,
    /// Extra identifiers the caller attached, such as the cluster and the member
    /// that answered. Never request content.
    pub detail: Option<serde_json::Value>,
}

/// One stored request, as the accounting list shows it.
///
/// The content digest is the only trace of the request body, and it cannot be
/// reversed: the table it comes from has no column for the content.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct AccountingRow {
    /// The row's id.
    pub id: Uuid,
    /// The tenant the row belongs to.
    pub environment_id: Uuid,
    /// The sandbox that made the request.
    pub sandbox_id: Uuid,
    /// Which agent inside the sandbox made it, when known.
    pub agent_instance: Option<String>,
    /// The provider the request went to (or came from).
    pub provider: Option<String>,
    /// The model the provider named.
    pub model: Option<String>,
    /// Which way the request travelled.
    pub direction: Direction,
    /// The endpoint the proxy saw.
    pub endpoint: Option<String>,
    /// The HTTP status the proxy returned.
    pub status: i32,
    /// How long the request took, in milliseconds.
    pub latency_ms: i32,
    /// The tokens the provider reported.
    pub usage: Usage,
    /// SHA-256 of the content, lowercase hex.
    pub content_digest: Option<String>,
    /// `None` when no price was known; never zero as a stand-in.
    pub cost_micros: Option<i64>,
    /// Where the cost came from.
    pub price_source: PriceSource,
    /// What the privacy layer did, when it ran.
    pub privacy_action: Option<String>,
    /// Which detector decided it, when one did.
    pub privacy_detector: Option<String>,
    /// When the proxy recorded it.
    pub created_at: DateTime<Utc>,
}

/// One model's share of a window.
///
/// `cost_micros` is `None` when no request in the group had a price, and the
/// overview reports how many requests were unpriced, so a partial total cannot
/// be mistaken for a complete one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct AccountingSummary {
    /// The model name, or `unknown` when the request named none.
    pub model: String,
    /// How many requests the group holds.
    pub requests: i64,
    /// The prompt tokens the group used.
    pub prompt_tokens: i64,
    /// The completion tokens the group used.
    pub completion_tokens: i64,
    /// The part of `prompt_tokens` the provider served from cache.
    pub cached_tokens: i64,
    /// `None` when not one request in the group had a price.
    pub cost_micros: Option<i64>,
}

/// One sandbox's share of a window.
///
/// The name is read from `sandboxes` and falls back to the id, because a rollup
/// outlives the sandbox it describes and a deleted sandbox must still be named
/// in an old window.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct SandboxSummary {
    /// The sandbox that made the requests.
    pub sandbox_id: Uuid,
    /// The sandbox's name, or its id when the row no longer exists.
    pub sandbox_name: String,
    /// How many requests the sandbox made in the window.
    pub requests: i64,
    /// `None` when not one of its requests had a price.
    pub cost_micros: Option<i64>,
}

/// One privacy verdict and how often it was reached in the window.
///
/// Only rows that carry a verdict are counted: "we did not inspect this" is not
/// an action, and folding it into `allowed` would read as a pass.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct PrivacyStat {
    /// `allowed`, `replaced`, `blocked`, `not_inspectable`, …
    pub action: String,
    /// How many requests ended with it.
    pub count: i64,
}

/// The whole window in one row, so the console does not have to add up its
/// groups (and cannot accidentally display a total that excludes an unpriced
/// model).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, ToSchema)]
pub struct Totals {
    /// Every request in the window.
    pub requests: i64,
    /// Every prompt token in the window.
    pub prompt_tokens: i64,
    /// Every completion token in the window.
    pub completion_tokens: i64,
    /// The part of `prompt_tokens` served from cache.
    pub cached_tokens: i64,
    /// `None` when the window had no priced request at all.
    pub cost_micros: Option<i64>,
    /// How many requests had no price and therefore no cost.
    pub price_missing_requests: i64,
}

/// The answer to an overview request: totals, per model, per sandbox, and the
/// privacy verdicts of the same window.
///
/// `sandboxes` is empty for a sandbox-scoped overview, because a one-row
/// breakdown is noise. `price_missing_requests` is repeated from `totals` so a
/// headline can say "this total covers 340 of 400 requests" without reading the
/// totals row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct Overview {
    /// The window the caller asked for.
    pub timeframe: Timeframe,
    /// The bound the numbers were queried with.
    pub since: DateTime<Utc>,
    /// Per model, most expensive first.
    pub summaries: Vec<AccountingSummary>,
    /// Per sandbox, most expensive first; empty for a sandbox-scoped overview.
    pub sandboxes: Vec<SandboxSummary>,
    /// What the privacy layer did in the window.
    pub privacy: Vec<PrivacyStat>,
    /// The window in one row.
    pub totals: Totals,
    /// How many requests in the window had no price.
    pub price_missing_requests: i64,
}

/// How long raw accounting rows are kept, and whether the rollup outlives them.
///
/// A missing settings row means the defaults here; the service answers with this
/// value rather than a 404, because "never configured" and "configured to the
/// default" are the same policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct AccountingSettings {
    /// Days of raw rows to keep, between [`MIN_RETENTION_DAYS`] and
    /// [`MAX_RETENTION_DAYS`].
    pub retention_days: i32,
    /// Whether the daily rollup is kept when raw rows are purged.
    pub keep_daily: bool,
    /// Who saved the setting last, when anyone has.
    pub updated_by: Option<Uuid>,
    /// When it was saved last, when it ever was.
    pub updated_at: Option<DateTime<Utc>>,
}

impl Default for AccountingSettings {
    fn default() -> Self {
        Self {
            retention_days: DEFAULT_RETENTION_DAYS,
            keep_daily: true,
            updated_by: None,
            updated_at: None,
        }
    }
}

impl AccountingSettings {
    /// Whether the retention window is inside the documented range.
    ///
    /// The database CHECK repeats it; this is the version a caller can be told
    /// about in a sentence instead of a constraint name.
    pub fn invalid_reason(&self) -> Option<String> {
        if !(MIN_RETENTION_DAYS..=MAX_RETENTION_DAYS).contains(&self.retention_days) {
            return Some(format!(
                "retention_days must be between {MIN_RETENTION_DAYS} and {MAX_RETENTION_DAYS}."
            ));
        }
        None
    }

    /// The moment raw rows older than this may be removed.
    pub fn cutoff(&self, now: DateTime<Utc>) -> DateTime<Utc> {
        now - Duration::days(i64::from(self.retention_days))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn price() -> Price {
        Price {
            input_per_mtok: 3_000_000,
            output_per_mtok: 15_000_000,
            cached_per_mtok: Some(300_000),
        }
    }

    #[test]
    fn cost_is_exact_in_micros() {
        let usage = Usage::new(1_000, 500, 0, 0);
        // 1000 tokens * 3.00/Mtok = 3000 micros; 500 * 15.00/Mtok = 7500.
        assert_eq!(cost_micros(&usage, Some(&price())), Some(10_500));
    }

    #[test]
    fn cached_tokens_are_billed_at_the_cached_rate() {
        let cached = Usage::new(1_000_000, 0, 1_000_000, 0);
        let uncached = Usage::new(1_000_000, 0, 0, 0);
        assert_eq!(cost_micros(&cached, Some(&price())), Some(300_000));
        assert_eq!(cost_micros(&uncached, Some(&price())), Some(3_000_000));
    }

    #[test]
    fn cached_tokens_are_cheaper_than_the_same_input_tokens() {
        let with_cache = Usage::new(1_000_000, 0, 800_000, 0);
        let without_cache = Usage::new(1_000_000, 0, 0, 0);
        let cached_cost = cost_micros(&with_cache, Some(&price()));
        let plain_cost = cost_micros(&without_cache, Some(&price()));
        assert_eq!(cached_cost, Some(840_000)); // 200k input + 800k cached
        assert!(cached_cost < plain_cost);
    }

    #[test]
    fn a_price_without_a_cached_rate_charges_input() {
        let price = Price {
            cached_per_mtok: None,
            ..price()
        };
        let usage = Usage::new(1_000_000, 0, 1_000_000, 0);
        assert_eq!(cost_micros(&usage, Some(&price)), Some(3_000_000));
    }

    #[test]
    fn missing_price_is_none_not_zero() {
        assert_eq!(cost_micros(&Usage::new(10, 10, 0, 0), None), None);
    }

    #[test]
    fn reasoning_tokens_are_not_billed_twice() {
        let without = Usage::new(100, 100, 0, 0);
        let with = Usage::new(100, 100, 0, 80);
        assert_eq!(
            cost_micros(&without, Some(&price())),
            cost_micros(&with, Some(&price()))
        );
    }

    #[test]
    fn a_sub_micro_cost_rounds_up() {
        let price = Price {
            input_per_mtok: 500_000,
            output_per_mtok: 500_000,
            cached_per_mtok: None,
        };
        assert_eq!(cost_micros(&Usage::new(1, 0, 0, 0), Some(&price)), Some(1));
    }

    #[test]
    fn usage_new_clamps_a_cached_count_above_the_prompt() {
        let usage = Usage::new(-5, -1, 10, -2);
        assert_eq!(usage, Usage::new(0, 0, 0, 0));
        let usage = Usage::new(100, 20, 400, 5);
        assert_eq!(usage.cached, 100);
        assert_eq!(usage.billable_prompt(), 0);
    }

    #[test]
    fn timeframes_round_trip_and_measure() {
        for (frame, text, seconds) in [
            (Timeframe::M5, "5m", 300),
            (Timeframe::H1, "1h", 3_600),
            (Timeframe::H24, "24h", 86_400),
            (Timeframe::D7, "7d", 604_800),
            (Timeframe::D30, "30d", 2_592_000),
        ] {
            assert_eq!(frame.as_str(), text);
            assert_eq!(Timeframe::parse(text), Some(frame));
            assert_eq!(frame.seconds(), seconds);
        }
        assert_eq!(Timeframe::parse("2h"), None);
        assert_eq!(Timeframe::parse(""), None);
        assert_eq!(Timeframe::default(), Timeframe::H24);
    }

    #[test]
    fn a_timeframe_bound_is_now_minus_its_length() {
        let now = DateTime::UNIX_EPOCH + Duration::days(10);
        assert_eq!(
            Timeframe::H24.since(now),
            now - Duration::seconds(86_400)
        );
    }

    #[test]
    fn price_sources_round_trip_and_reject_unknown() {
        for source in [PriceSource::Catalog, PriceSource::Provider, PriceSource::Unknown] {
            assert_eq!(PriceSource::parse(source.as_str()), Some(source));
        }
        assert_eq!(PriceSource::parse("guessed"), None);
    }

    #[test]
    fn directions_accept_both_spellings_and_store_the_short_one() {
        assert_eq!(Direction::parse("in"), Some(Direction::In));
        assert_eq!(Direction::parse("out"), Some(Direction::Out));
        assert_eq!(Direction::parse("inbound"), Some(Direction::In));
        assert_eq!(Direction::parse("outbound"), Some(Direction::Out));
        assert_eq!(Direction::In.as_str(), "in");
        assert_eq!(Direction::Out.as_str(), "out");
        assert_eq!(Direction::parse("sideways"), None);
    }

    #[test]
    fn a_default_settings_row_keeps_thirty_days() {
        let settings = AccountingSettings::default();
        assert_eq!(settings.retention_days, DEFAULT_RETENTION_DAYS);
        assert!(settings.keep_daily);
        assert_eq!(settings.invalid_reason(), None);
    }

    #[test]
    fn a_retention_outside_the_range_names_the_range() {
        let settings = AccountingSettings {
            retention_days: 0,
            ..AccountingSettings::default()
        };
        let reason = settings.invalid_reason();
        assert!(reason.is_some());
        assert!(reason.unwrap_or_default().contains("retention_days"));
        assert!(AccountingSettings {
            retention_days: MAX_RETENTION_DAYS + 1,
            ..AccountingSettings::default()
        }
        .invalid_reason()
        .is_some());
    }

    #[test]
    fn a_cutoff_is_now_minus_the_retention() {
        let settings = AccountingSettings::default();
        let now = DateTime::UNIX_EPOCH + Duration::days(100);
        assert_eq!(
            settings.cutoff(now),
            now - Duration::days(i64::from(DEFAULT_RETENTION_DAYS))
        );
    }

    #[test]
    fn a_negative_rate_is_refused() {
        assert_eq!(price().invalid_reason(), None);
        assert!(Price {
            input_per_mtok: -1,
            ..price()
        }
        .invalid_reason()
        .is_some());
        assert!(Price {
            cached_per_mtok: Some(-1),
            ..price()
        }
        .invalid_reason()
        .is_some());
    }
}


// ---------------------------------------------------------------------------
// Usage read off the transparent proxy
// ---------------------------------------------------------------------------

/// Whose bill a model call is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AuthKind {
    /// An API key: the call has a cost.
    Key,
    /// A signed-in account (Claude, Codex): the subscription is the price, so the
    /// call has no cost, only an estimate at list price.
    Account,
    #[default]
    Unknown,
}

impl AuthKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Key => "key",
            Self::Account => "account",
            Self::Unknown => "unknown",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "key" => Self::Key,
            "account" => Self::Account,
            _ => Self::Unknown,
        }
    }
}

/// The counters the proxy read from one answer, already in one vocabulary.
///
/// `input` is the *fresh* prompt: what the model read that was not served from the
/// cache and not written to it. The providers disagree about this (Anthropic's
/// `input_tokens` excludes the cache, OpenAI's `prompt_tokens` includes it); the
/// proxy normalises before it ships, so everything here adds up the same way.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ObservedUsage {
    pub input: i64,
    pub output: i64,
    pub cache_read: i64,
    /// All cache writes, whichever lifetime.
    pub cache_write: i64,
    /// The part of `cache_write` kept for an hour (billed higher than five minutes).
    pub cache_write_1h: i64,
    pub reasoning: i64,
}

impl ObservedUsage {
    /// Everything the model read.
    pub fn prompt(&self) -> i64 {
        self.input.max(0) + self.cache_read.max(0) + self.cache_write.max(0)
    }

    /// Clamp into the shape the schema accepts.
    pub fn normalised(self) -> Self {
        let cache_write = self.cache_write.max(0);
        Self {
            input: self.input.max(0),
            output: self.output.max(0),
            cache_read: self.cache_read.max(0),
            cache_write,
            cache_write_1h: self.cache_write_1h.clamp(0, cache_write),
            reasoning: self.reasoning.clamp(0, self.output.max(0)),
        }
    }

    /// The share of the prompt that came out of the cache, 0..=1.
    pub fn cache_hit_rate(&self) -> f64 {
        let prompt = self.prompt();
        if prompt == 0 { 0.0 } else { self.cache_read.max(0) as f64 / prompt as f64 }
    }
}

/// What a call costs at `price`, cache reads and writes included.
///
/// Writes cost more than input (Anthropic: 1.25x for five minutes, 2x for an hour)
/// and reads cost less (the price's cached rate). The write multipliers are the
/// provider's published ratios, not a stored rate: a price row carries input,
/// output and cached, and the writes follow from the input rate.
pub fn observed_cost_micros(usage: &ObservedUsage, price: Option<&Price>) -> Option<i64> {
    let price = price?;
    let u = usage.normalised();
    let input_rate = price.input_per_mtok.max(0) as i128;
    let write_5m = (u.cache_write - u.cache_write_1h) as i128;
    let micros = (u.input as i128) * input_rate
        + (u.cache_read as i128) * (price.cached_rate().max(0) as i128)
        + write_5m * input_rate * 5 / 4
        + (u.cache_write_1h as i128) * input_rate * 2
        + (u.output as i128) * (price.output_per_mtok.max(0) as i128);
    Some(round_to_micros(micros))
}

#[cfg(test)]
mod observed_tests {
    use super::*;

    fn price() -> Price {
        Price { input_per_mtok: 3_000_000, output_per_mtok: 15_000_000, cached_per_mtok: Some(300_000) }
    }

    #[test]
    fn a_call_is_priced_with_reads_and_writes() {
        // 1,000,000 fresh in, 1,000,000 cache reads, 1,000,000 five-minute writes,
        // 1,000,000 one-hour writes, 1,000,000 out.
        let usage = ObservedUsage {
            input: 1_000_000,
            output: 1_000_000,
            cache_read: 1_000_000,
            cache_write: 2_000_000,
            cache_write_1h: 1_000_000,
            reasoning: 0,
        };
        // 3.00 + 0.30 + 3.75 + 6.00 + 15.00 = 28.05 dollars.
        assert_eq!(observed_cost_micros(&usage, Some(&price())), Some(28_050_000));
        assert_eq!(observed_cost_micros(&usage, None), None);
    }

    #[test]
    fn the_prompt_adds_up_and_the_hit_rate_is_a_share_of_it() {
        let usage = ObservedUsage { input: 1_997, output: 81, cache_read: 42_537, cache_write: 535, cache_write_1h: 535, reasoning: 79 };
        assert_eq!(usage.prompt(), 45_069);
        assert!((usage.cache_hit_rate() - 42_537.0 / 45_069.0).abs() < 1e-9);
        assert_eq!(ObservedUsage::default().cache_hit_rate(), 0.0);
    }

    #[test]
    fn nonsense_is_clamped_not_stored() {
        let usage = ObservedUsage { input: -5, output: 10, cache_read: 0, cache_write: 3, cache_write_1h: 99, reasoning: 50 }.normalised();
        assert_eq!((usage.input, usage.cache_write_1h, usage.reasoning), (0, 3, 10));
    }

    #[test]
    fn auth_round_trips() {
        for kind in [AuthKind::Key, AuthKind::Account, AuthKind::Unknown] {
            assert_eq!(AuthKind::parse(kind.as_str()), kind);
        }
        assert_eq!(AuthKind::parse("anything"), AuthKind::Unknown);
    }
}
