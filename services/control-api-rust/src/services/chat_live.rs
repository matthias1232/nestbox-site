//! The live report of a model request: it started, its answer grows, it is done.
//!
//! Nothing here is stored. The broker and the accounting proxy report as they
//! stream; the control API masks the text and publishes it on the live bus. The
//! stored turn is written later, from the access log.
//!
//! A report is throttled per request, so a fast answer cannot flood the bus.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::Deserialize;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::domain::telemetry::LiveEvent;
use crate::error::ApiError;
use crate::services::ServiceContext;

/// The shortest gap between two reports of the same request.
const MIN_GAP: Duration = Duration::from_millis(250);
/// How long a request may be silent before its entry is forgotten.
const FORGET: Duration = Duration::from_secs(600);
/// Longest answer text a live report carries, in bytes.
const LIVE_TEXT_MAX: usize = 16 * 1024;
/// Longest user text a start report carries, in bytes.
const LIVE_USER_MAX: usize = 2048;

/// How a turn started, for callers that do not go through the content check.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct LiveStart {
    pub model: Option<String>,
    pub host: Option<String>,
    pub account_kind: Option<String>,
    #[serde(default)]
    pub user_text: String,
}

/// One live report from the broker or the accounting proxy.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct LiveChunk {
    pub request_ref: String,
    pub sandbox_id: Uuid,
    #[serde(default)]
    pub text_so_far: String,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    #[serde(default)]
    pub done: bool,
    #[serde(default)]
    pub started: Option<LiveStart>,
}

fn last_report() -> &'static Mutex<HashMap<String, Instant>> {
    static LAST: OnceLock<Mutex<HashMap<String, Instant>>> = OnceLock::new();
    LAST.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Whether a report should be published now.
///
/// The last report of a request is always published; otherwise at most one every
/// [`MIN_GAP`]. A missing or poisoned lock publishes, because dropping the last
/// token of an answer is worse than an extra event.
pub fn should_publish(last: Option<Instant>, now: Instant, done: bool) -> bool {
    match last {
        None => true,
        Some(at) => done || now.duration_since(at) >= MIN_GAP,
    }
}

/// Accept one report, mask it and publish it.
///
/// A report is untrusted in one respect: it carries text. The text is masked here,
/// with the same policy the request will be checked against, so a live viewer
/// cannot see more than the provider did.
pub async fn accept(ctx: &ServiceContext, chunk: LiveChunk) -> Result<(), ApiError> {
    let reference = chunk.request_ref.trim();
    if reference.is_empty() || reference.chars().count() > 100 {
        return Err(ApiError::validation(
            "request_ref must be 1 to 100 characters.",
        ));
    }
    let environment = match crate::store::postgres::egress::sandbox_environment(
        &ctx.pool,
        chunk.sandbox_id,
    )
    .await
    .map_err(ApiError::from)?
    {
        Some(environment) => environment,
        None => {
            return Err(ApiError::not_found(
                crate::error::ErrorCode::NOT_FOUND,
                "No such sandbox in this environment.",
            ))
        }
    };

    if let Some(start) = &chunk.started {
        let user_text = mask(ctx, environment, chunk.sandbox_id, &start.user_text).await;
        ctx.bus.publish(LiveEvent::ChatStarted {
            environment_id: environment,
            sandbox_id: chunk.sandbox_id,
            request_ref: reference.to_string(),
            ts: chrono::Utc::now(),
            model: start.model.clone(),
            host: start.host.clone(),
            provider: crate::domain::chat_turn::provider_for(
                start.host.as_deref().unwrap_or_default(),
            )
            .to_string(),
            account_kind: start.account_kind.clone(),
            dlp: None,
            delivered: true,
            user_text,
        });
    }

    let now = Instant::now();
    let previous = {
        let mut guard = match last_report().lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        guard.retain(|_, at| now.duration_since(*at) < FORGET);
        let previous = guard.get(reference).copied();
        if should_publish(previous, now, chunk.done) || chunk.started.is_some() {
            guard.insert(reference.to_string(), now);
            if chunk.done {
                guard.remove(reference);
            }
            previous
        } else {
            return Ok(());
        }
    };
    let _ = previous;

    let text = mask(ctx, environment, chunk.sandbox_id, &chunk.text_so_far).await;
    ctx.bus.publish(LiveEvent::ChatDelta {
        environment_id: environment,
        sandbox_id: chunk.sandbox_id,
        request_ref: reference.to_string(),
        text_so_far: crate::domain::chat_turn::truncate_chars(&text, LIVE_TEXT_MAX),
        input_tokens: chunk.input_tokens,
        output_tokens: chunk.output_tokens,
        done: chunk.done,
    });
    Ok(())
}

/// Mask a live text with the sandbox's own policy.
///
/// A failure returns the text empty rather than raw: a live view that cannot be
/// masked must show nothing, not everything.
async fn mask(ctx: &ServiceContext, environment: Uuid, sandbox_id: Uuid, text: &str) -> String {
    if text.is_empty() {
        return String::new();
    }
    match crate::services::privacy_service::scrub_batch(
        ctx,
        environment,
        &[(sandbox_id, text.to_string())],
    )
    .await
    {
        Ok(mut masked) if !masked.is_empty() => masked.remove(0),
        _ => String::new(),
    }
}

/// Longest answer text a report may carry.
pub(crate) fn live_text_max() -> usize {
    LIVE_TEXT_MAX
}

/// Longest user text a start report may carry.
pub(crate) fn live_user_max() -> usize {
    LIVE_USER_MAX
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_report_goes_out_immediately() {
        assert!(should_publish(None, Instant::now(), false));
    }

    #[test]
    fn a_burst_is_throttled_but_the_last_report_is_not() {
        let start = Instant::now();
        assert!(!should_publish(Some(start), start + Duration::from_millis(50), false));
        assert!(should_publish(Some(start), start + Duration::from_millis(300), false));
        // `done` is always published, however soon it follows.
        assert!(should_publish(Some(start), start + Duration::from_millis(1), true));
    }
}
