//! Sign-in through an identity provider (ADR 0046).

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Redirect, Response};
use serde::Deserialize;

use crate::api::auth::build_session_cookie;
use crate::error::ApiError;
use crate::services::oidc_service::{self, LoginOptions};
use crate::services::auth_service;
use crate::state::AppState;

/// The sign-in methods on offer, for the login page.
pub async fn options(State(state): State<AppState>) -> Json<LoginOptions> {
    Json(oidc_service::options(&state.config))
}

#[derive(Debug, Deserialize)]
pub struct StartQuery {
    #[serde(default)]
    pub invitation: Option<String>,
}

/// Send the browser to the identity provider.
pub async fn start(
    State(state): State<AppState>,
    Path(provider): Path<String>,
    Query(query): Query<StartQuery>,
) -> Result<Redirect, ApiError> {
    let url = oidc_service::start(&state.services, &state.config, &provider, query.invitation.as_deref()).await?;
    Ok(Redirect::to(&url))
}

#[derive(Debug, Deserialize)]
pub struct CallbackQuery {
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

/// Where the identity provider sends the person back. On success the session cookie is set
/// and the browser lands on the console; on failure it lands there with a short reason.
pub async fn callback(
    State(state): State<AppState>,
    Path(provider): Path<String>,
    Query(query): Query<CallbackQuery>,
) -> Response {
    match finish(&state, &provider, query).await {
        Ok(cookie) => {
            let mut headers = HeaderMap::new();
            headers.insert(header::SET_COOKIE, cookie);
            headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            headers.insert(header::LOCATION, HeaderValue::from_static("/"));
            (StatusCode::SEE_OTHER, headers).into_response()
        }
        Err(error) => {
            let reason: String = error.message.chars().take(160).collect();
            let location = format!("/?login_error={}", percent(&reason));
            Redirect::to(&location).into_response()
        }
    }
}

async fn finish(state: &AppState, provider: &str, query: CallbackQuery) -> Result<HeaderValue, ApiError> {
    if let Some(error) = query.error {
        return Err(ApiError::unauthenticated(format!("The identity provider reported: {error}")));
    }
    let (Some(code), Some(token_state)) = (query.code, query.state) else {
        return Err(ApiError::unauthenticated("The sign-in answer was incomplete."));
    };
    let (user_id, name) = oidc_service::complete(&state.services, &state.config, provider, &code, &token_state).await?;
    let result = auth_service::start_session(
        state.pool(),
        user_id,
        name,
        state.config.session_idle_seconds,
        state.config.session_max_seconds,
    )
    .await?;
    Ok(build_session_cookie(
        &result.token,
        state.config.session_max_seconds,
        state.config.session_cookie_secure,
    ))
}

/// Percent-encode for a query value. Small and total, so no extra crate for one use.
fn percent(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.~".contains(&byte) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reason_is_safe_in_a_query_string() {
        assert_eq!(percent("a b&c=d"), "a%20b%26c%3Dd");
        assert_eq!(percent("ok-1_2.3~"), "ok-1_2.3~");
    }
}
