//! Authentication: login, session validation, principal construction.

use chrono::{Duration, Utc};
use uuid::Uuid;

use crate::domain::auth::{CredentialSource, Principal, Role};
use crate::error::{ApiError, ErrorCode};
use crate::security::password::{is_usable_hash, verify_password};
use crate::security::session::{self, hashes_equal};
use crate::store::postgres::auth as auth_store;
use crate::store::postgres::environments as env_store;

/// The result of a successful login.
pub struct LoginResult {
    pub token: String,
    pub csrf_token: String,
    pub environment_id: Uuid,
    pub role: Role,
    pub display_name: String,
    pub environments: Vec<(Uuid, String, Role)>,
}

/// The outcome of a login attempt, with a deliberately uniform failure.
///
/// A wrong email and a wrong password produce the same error, so the endpoint
/// cannot be used to enumerate accounts.
pub async fn login(
    pool: &sqlx::PgPool,
    email: &str,
    password: &str,
    idle_seconds: i64,
    max_seconds: i64,
) -> Result<LoginResult, ApiError> {
    let Some(user) = auth_store::find_user_by_email(pool, email)
        .await
        .map_err(ApiError::from)?
    else {
        // Same shape and roughly the same cost as a failed password check, so
        // timing does not distinguish "no such user" from "wrong password".
        let _ = verify_password(password.to_string(), "$argon2id$v=19$m=19456,t=2,p=1$x$y".into()).await;
        return Err(invalid_credentials());
    };

    if user.status != "active" || !is_usable_hash(&user.password_hash) {
        return Err(invalid_credentials());
    }

    if !verify_password(password.to_string(), user.password_hash.clone()).await {
        return Err(invalid_credentials());
    }

    start_session(pool, user.id, user.display_name, idle_seconds, max_seconds).await
}

/// Open a session for a user whose identity was already proven, by password or by an
/// identity provider. Everything after "who is this" is the same either way.
pub async fn start_session(
    pool: &sqlx::PgPool,
    user_id: Uuid,
    display_name: String,
    idle_seconds: i64,
    max_seconds: i64,
) -> Result<LoginResult, ApiError> {
    let environments = auth_store::environments_for_user(pool, user_id)
        .await
        .map_err(ApiError::from)?;

    let Some((environment_id, _name, role)) = environments.first().cloned() else {
        return Err(ApiError::unauthenticated(
            "This account has no environment membership.",
        ));
    };

    let issued = session::issue();
    let now = Utc::now();
    let expires_at = now + Duration::seconds(max_seconds);
    let idle_deadline = now + Duration::seconds(idle_seconds);

    auth_store::insert_session(
        pool,
        user_id,
        &issued.token_hash,
        &issued.csrf_hash,
        expires_at,
        idle_deadline,
    )
    .await
    .map_err(ApiError::from)?;

    Ok(LoginResult {
        token: issued.token,
        csrf_token: issued.csrf_token,
        environment_id,
        role,
        display_name,
        environments,
    })
}

fn invalid_credentials() -> ApiError {
    ApiError::new(
        axum::http::StatusCode::UNAUTHORIZED,
        ErrorCode::UNAUTHENTICATED,
        "Email or password is incorrect.",
    )
}

/// Resolve a principal from a session token.
pub async fn principal_from_session(
    pool: &sqlx::PgPool,
    token: &str,
    idle_seconds: i64,
) -> Result<(Principal, String), ApiError> {
    let hash = session::hash_token(token);
    let Some(record) = auth_store::find_session(pool, &hash)
        .await
        .map_err(ApiError::from)?
    else {
        return Err(ApiError::new(
            axum::http::StatusCode::UNAUTHORIZED,
            ErrorCode::SESSION_EXPIRED,
            "The session is missing or has expired.",
        ));
    };

    if record.status != "active" {
        return Err(ApiError::unauthenticated("This account is disabled."));
    }

    // The environment is the user's first membership. A multi-environment
    // switcher passes a different id explicitly and is verified in
    // `principal_for_environment`.
    let environments = auth_store::environments_for_user(pool, record.user_id)
        .await
        .map_err(ApiError::from)?;
    let Some((environment_id, _name, role)) = environments.first().cloned() else {
        return Err(ApiError::unauthenticated(
            "This account has no environment membership.",
        ));
    };

    let idle_deadline = Utc::now() + Duration::seconds(idle_seconds);
    auth_store::touch_session(pool, record.id, idle_deadline)
        .await
        .map_err(ApiError::from)?;

    Ok((
        Principal {
            user_id: record.user_id,
            environment_id,
            role,
            display_name: record.display_name,
            via: CredentialSource::Session,
        },
        record.csrf_hash,
    ))
}

/// Resolve a principal from an API token.
pub async fn principal_from_token(
    pool: &sqlx::PgPool,
    token: &str,
) -> Result<Principal, ApiError> {
    let hash = session::hash_token(token);
    let Some(user_id) = auth_store::find_api_token_user(pool, &hash)
        .await
        .map_err(ApiError::from)?
    else {
        return Err(ApiError::unauthenticated("The token is invalid or revoked."));
    };

    let environments = auth_store::environments_for_user(pool, user_id)
        .await
        .map_err(ApiError::from)?;
    let Some((environment_id, _name, role)) = environments.first().cloned() else {
        return Err(ApiError::unauthenticated(
            "This token has no environment membership.",
        ));
    };

    Ok(Principal {
        user_id,
        environment_id,
        role,
        display_name: format!("token:{user_id}"),
        via: CredentialSource::Token,
    })
}

/// Verify a CSRF token against the stored hash.
pub fn verify_csrf(provided: &str, stored_hash: &str) -> Result<(), ApiError> {
    let provided_hash = session::hash_token(provided);
    if hashes_equal(&provided_hash, stored_hash) {
        Ok(())
    } else {
        Err(ApiError::new(
            axum::http::StatusCode::FORBIDDEN,
            ErrorCode::CSRF_REQUIRED,
            "A valid CSRF token is required for this request.",
        ))
    }
}

/// Log out: delete the session so it cannot be reused.
pub async fn logout(pool: &sqlx::PgPool, token: &str) -> Result<(), ApiError> {
    let hash = session::hash_token(token);
    auth_store::delete_session(pool, &hash)
        .await
        .map_err(ApiError::from)
}

/// Switch the active environment, verifying membership first.
pub async fn principal_for_environment(
    pool: &sqlx::PgPool,
    user_id: Uuid,
    display_name: &str,
    environment_id: Uuid,
    via: CredentialSource,
) -> Result<Principal, ApiError> {
    let role = auth_store::role_in_environment(pool, user_id, environment_id)
        .await
        .map_err(ApiError::from)?
        .ok_or_else(|| {
            ApiError::new(
                axum::http::StatusCode::FORBIDDEN,
                ErrorCode::FORBIDDEN,
                "You are not a member of that environment.",
            )
        })?;

    // Touch the environment row so an unknown id is a 404 rather than a silent
    // empty result set.
    let _ = env_store::get_environment(pool, environment_id)
        .await
        .map_err(ApiError::from)?;

    Ok(Principal {
        user_id,
        environment_id,
        role,
        display_name: display_name.to_string(),
        via,
    })
}

/// Give a signed-in browser a fresh CSRF token.
///
/// The token is memory-only in the console (storage would make it readable by any script
/// on the page), so a reload loses it, and only its hash is kept here. This hands out a
/// replacement for a live session, and the old token stops working. It needs the session
/// cookie, which is `SameSite=Lax` and so not sent on a cross-site POST, and the token is
/// returned only in the response body, which another origin cannot read.
pub async fn rotate_csrf(pool: &sqlx::PgPool, session_token: &str) -> Result<String, ApiError> {
    let fresh = session::issue_csrf_token();
    let rotated = auth_store::rotate_csrf(pool, &session::hash_token(session_token), &session::hash_token(&fresh))
        .await
        .map_err(ApiError::from)?;
    if !rotated {
        return Err(ApiError::new(
            axum::http::StatusCode::UNAUTHORIZED,
            ErrorCode::SESSION_EXPIRED,
            "The session is missing or has expired.",
        ));
    }
    Ok(fresh)
}
