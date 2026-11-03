//! Login, logout, and the current identity.

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::IntoResponse;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::api::principal::Auth;
use crate::error::ApiError;
use crate::security::session::SESSION_COOKIE;
use crate::services::auth_service;
use crate::state::AppState;

#[derive(Debug, Deserialize, ToSchema)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct LoginResponse {
    pub display_name: String,
    pub role: String,
    pub environment_id: uuid::Uuid,
    pub environments: Vec<EnvironmentChoice>,
    /// The CSRF token the client must echo on every mutating request. It is
    /// returned here once and held in memory by the console, never in storage.
    pub csrf_token: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct EnvironmentChoice {
    pub id: uuid::Uuid,
    pub name: String,
    pub role: String,
}

/// Exchange credentials for a session.
pub async fn login(
    State(state): State<AppState>,
    Json(body): Json<LoginRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let result = auth_service::login(
        state.pool(),
        body.email.trim(),
        &body.password,
        state.config.session_idle_seconds,
        state.config.session_max_seconds,
    )
    .await?;

    let cookie = build_session_cookie(
        &result.token,
        state.config.session_max_seconds,
        state.config.session_cookie_secure,
    );

    let response = LoginResponse {
        display_name: result.display_name,
        role: result.role.as_str().to_string(),
        environment_id: result.environment_id,
        environments: result
            .environments
            .into_iter()
            .map(|(id, name, role)| EnvironmentChoice {
                id,
                name,
                role: role.as_str().to_string(),
            })
            .collect(),
        csrf_token: result.csrf_token,
    };

    let mut headers = HeaderMap::new();
    headers.insert(header::SET_COOKIE, cookie);
    // A login response must never be cached: it carries a credential.
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-store"),
    );

    Ok((StatusCode::OK, headers, Json(response)))
}

/// Revoke the session.
pub async fn logout(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    if let Some(token) = crate::api::principal::session_token_from(&headers) {
        auth_service::logout(state.pool(), &token).await?;
    }

    let mut response_headers = HeaderMap::new();
    response_headers.insert(
        header::SET_COOKIE,
        HeaderValue::from_static(
            "nestbox_session=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0",
        ),
    );
    Ok((StatusCode::NO_CONTENT, response_headers))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CsrfResponse {
    pub csrf_token: String,
}

/// A fresh CSRF token for the current session, after a page reload lost the old one.
/// The previous token stops working.
pub async fn csrf(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    let Some(token) = crate::api::principal::session_token_from(&headers) else {
        return Err(ApiError::unauthenticated("Sign in first."));
    };
    let csrf_token = auth_service::rotate_csrf(state.pool(), &token).await?;
    let mut response_headers = HeaderMap::new();
    response_headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok((StatusCode::OK, response_headers, Json(CsrfResponse { csrf_token })))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MeResponse {
    pub display_name: String,
    pub role: String,
    pub environment_id: uuid::Uuid,
    pub user_id: uuid::Uuid,
}

/// Who the caller is.
pub async fn me(auth: Auth) -> Json<MeResponse> {
    Json(MeResponse {
        display_name: auth.principal.display_name.clone(),
        role: auth.principal.role.as_str().to_string(),
        environment_id: auth.principal.environment_id,
        user_id: auth.principal.user_id,
    })
}

/// Build the session cookie.
///
/// `HttpOnly` so script cannot read it, `SameSite=Lax` so a cross-site form
/// cannot ride on it, and `Secure` when the console is served over TLS.
pub(crate) fn build_session_cookie(token: &str, max_age_seconds: i64, secure: bool) -> HeaderValue {
    let mut value = format!(
        "{SESSION_COOKIE}={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age_seconds}"
    );
    if secure {
        value.push_str("; Secure");
    }
    HeaderValue::from_str(&value)
        .unwrap_or_else(|_| HeaderValue::from_static("nestbox_session=; Max-Age=0"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cookie_is_http_only_and_same_site() {
        let cookie = build_session_cookie("abc", 3600, false);
        let text = cookie.to_str().expect("ascii");
        assert!(text.contains("HttpOnly"));
        assert!(text.contains("SameSite=Lax"));
        assert!(text.contains("Max-Age=3600"));
        assert!(!text.contains("Secure"));
    }

    #[test]
    fn the_cookie_is_secure_when_tls_is_on() {
        let cookie = build_session_cookie("abc", 3600, true);
        assert!(cookie.to_str().expect("ascii").contains("Secure"));
    }
}
