//! Sign-in through an identity provider (ADR 0046).
//!
//! The flow is the authorisation-code flow with PKCE. The server keeps the state and the
//! verifier (`oidc_states`), exchanges the code for an access token at the provider's token
//! endpoint, and reads the person's profile from the provider's userinfo endpoint over TLS.
//! Reading the profile that way, rather than trusting a token handed through the browser,
//! is why no signature check of an ID token is needed here.
//!
//! A person is matched by `(provider, subject)`, never by email alone. A first sign-in is
//! accepted in exactly two cases: with an invitation, or in `saas` mode, where it starts an
//! onboarding. In `selfhosted` mode nobody registers.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use utoipa::ToSchema;
use uuid::Uuid;

use super::ServiceContext;
use super::tenancy_service;
use crate::config::{Config, DeployMode, OidcProviderConfig};
use crate::domain::tenancy::OrganizationKind;
use crate::error::{ApiError, ErrorCode};
use crate::security::session;

/// What the login page needs to draw its buttons.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ProviderChoice {
    pub id: String,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct LoginOptions {
    pub providers: Vec<ProviderChoice>,
    /// `saas` or `selfhosted`.
    pub mode: String,
    /// Whether a person the system does not know can register.
    pub registration_open: bool,
}

/// The sign-in methods this installation offers.
pub fn options(config: &Config) -> LoginOptions {
    LoginOptions {
        providers: config.oidc.iter().map(|p| ProviderChoice { id: p.id.clone(), label: p.label.clone() }).collect(),
        mode: if config.deploy_mode == DeployMode::Saas { "saas" } else { "selfhosted" }.into(),
        registration_open: config.deploy_mode == DeployMode::Saas && !config.oidc.is_empty(),
    }
}

#[derive(Debug, Deserialize)]
struct Discovery {
    authorization_endpoint: String,
    token_endpoint: String,
    #[serde(default)]
    userinfo_endpoint: Option<String>,
}

struct Endpoints {
    authorize: String,
    token: String,
    userinfo: String,
    scope: &'static str,
}

fn client() -> Result<reqwest::Client, ApiError> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| ApiError::internal(e.to_string()))
}

fn unavailable(message: &str) -> ApiError {
    ApiError::unavailable(ErrorCode::UNAUTHENTICATED, message.to_string())
}

async fn endpoints(provider: &OidcProviderConfig) -> Result<Endpoints, ApiError> {
    if provider.kind == "github" {
        return Ok(Endpoints {
            authorize: "https://github.com/login/oauth/authorize".into(),
            token: "https://github.com/login/oauth/access_token".into(),
            userinfo: "https://api.github.com/user".into(),
            scope: "read:user user:email",
        });
    }
    let url = format!("{}/.well-known/openid-configuration", provider.issuer.trim_end_matches('/'));
    let found: Discovery = client()?
        .get(&url)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|_| unavailable("The identity provider cannot be reached right now."))?
        .json()
        .await
        .map_err(|_| unavailable("The identity provider answered something unexpected."))?;
    Ok(Endpoints {
        authorize: found.authorization_endpoint,
        token: found.token_endpoint,
        userinfo: found.userinfo_endpoint.ok_or_else(|| unavailable("The identity provider has no userinfo endpoint."))?,
        scope: "openid email profile",
    })
}

fn find<'a>(config: &'a Config, id: &str) -> Result<&'a OidcProviderConfig, ApiError> {
    config
        .oidc
        .iter()
        .find(|p| p.id == id)
        .ok_or_else(|| ApiError::not_found(ErrorCode::SANDBOX_NOT_FOUND, "identity provider"))
}

/// The redirect URI the provider sends the person back to. It must be registered at the
/// provider exactly as written.
pub fn redirect_uri(config: &Config, provider: &str) -> String {
    format!("{}/api/v1/auth/oidc/{provider}/callback", config.public_url)
}

/// The S256 challenge for a PKCE verifier.
pub fn challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// Begin a sign-in: remember the state and return the address to send the browser to.
pub async fn start(
    ctx: &ServiceContext,
    config: &Config,
    provider: &str,
    invitation_token: Option<&str>,
) -> Result<String, ApiError> {
    if config.public_url.is_empty() {
        return Err(unavailable("NESTBOX_PUBLIC_URL is not set, so the identity provider cannot send the person back."));
    }
    let provider = find(config, provider)?;
    let invitation_id: Option<Uuid> = match invitation_token {
        Some(token) => Some(
            sqlx::query_scalar(
                "SELECT id FROM invitations WHERE token_hash = $1 AND accepted_at IS NULL AND expires_at > now()",
            )
            .bind(session::hash_token(token))
            .fetch_optional(&ctx.pool)
            .await
            .map_err(|e| ApiError::from(crate::store::map_sqlx_error(e)))?
            .ok_or_else(|| ApiError::not_found(ErrorCode::SANDBOX_NOT_FOUND, "invitation"))?,
        ),
        None => None,
    };
    let endpoints = endpoints(provider).await?;
    let state = session::random_token();
    let verifier = session::random_token();
    sqlx::query("DELETE FROM oidc_states WHERE created_at < now() - interval '15 minutes'")
        .execute(&ctx.pool)
        .await
        .map_err(|e| ApiError::from(crate::store::map_sqlx_error(e)))?;
    sqlx::query("INSERT INTO oidc_states (state, provider, code_verifier, nonce, invitation_id) VALUES ($1, $2, $3, '', $4)")
        .bind(&state)
        .bind(&provider.id)
        .bind(&verifier)
        .bind(invitation_id)
        .execute(&ctx.pool)
        .await
        .map_err(|e| ApiError::from(crate::store::map_sqlx_error(e)))?;
    let mut url = reqwest::Url::parse(&endpoints.authorize).map_err(|_| unavailable("The provider's authorise address is invalid."))?;
    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", &provider.client_id)
        .append_pair("redirect_uri", &redirect_uri(config, &provider.id))
        .append_pair("scope", endpoints.scope)
        .append_pair("state", &state)
        .append_pair("code_challenge", &challenge(&verifier))
        .append_pair("code_challenge_method", "S256");
    Ok(url.to_string())
}

/// What the provider says about the person.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    pub subject: String,
    pub email: String,
    pub email_verified: bool,
    pub name: String,
}

/// Read a profile out of the userinfo answer. Pure, so the shapes providers really return
/// (a string or a number as subject, a missing name, GitHub's separate email list) are tested.
pub fn parse_profile(kind: &str, user: &serde_json::Value, github_emails: Option<&serde_json::Value>) -> Option<Profile> {
    let subject = match user.get(if kind == "github" { "id" } else { "sub" })? {
        serde_json::Value::String(s) if !s.is_empty() => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        _ => return None,
    };
    let (email, verified) = if kind == "github" {
        let list = github_emails?.as_array()?;
        let primary = list
            .iter()
            .find(|e| e["primary"].as_bool() == Some(true) && e["verified"].as_bool() == Some(true))?;
        (primary["email"].as_str()?.to_string(), true)
    } else {
        let verified = match user.get("email_verified") {
            Some(serde_json::Value::Bool(b)) => *b,
            Some(serde_json::Value::String(s)) => s == "true",
            _ => false,
        };
        (user["email"].as_str()?.to_string(), verified)
    };
    let name = user["name"]
        .as_str()
        .or_else(|| user["login"].as_str())
        .filter(|n| !n.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| email.split('@').next().unwrap_or("user").to_string());
    Some(Profile { subject, email: email.trim().to_lowercase(), email_verified: verified, name })
}

async fn fetch_profile(provider: &OidcProviderConfig, endpoints: &Endpoints, access_token: &str) -> Result<Profile, ApiError> {
    let http = client()?;
    let get = |url: &str| http.get(url).bearer_auth(access_token).header("User-Agent", "nestbox").header("Accept", "application/json");
    let user: serde_json::Value = get(&endpoints.userinfo)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|_| unavailable("The identity provider did not return a profile."))?
        .json()
        .await
        .map_err(|_| unavailable("The identity provider answered something unexpected."))?;
    let emails = if provider.kind == "github" {
        match get("https://api.github.com/user/emails").send().await {
            Ok(response) if response.status().is_success() => response.json::<serde_json::Value>().await.ok(),
            _ => None,
        }
    } else {
        None
    };
    parse_profile(&provider.kind, &user, emails.as_ref())
        .ok_or_else(|| ApiError::unauthenticated("The identity provider gave no verified email address."))
}

/// Finish a sign-in: exchange the code, find or create the person, and return their id.
pub async fn complete(
    ctx: &ServiceContext,
    config: &Config,
    provider_id: &str,
    code: &str,
    state: &str,
) -> Result<(Uuid, String), ApiError> {
    let provider = find(config, provider_id)?;
    // The state is single-use: taking it removes it, so a replayed callback finds nothing.
    let row: Option<(String, Option<Uuid>)> = sqlx::query_as(
        "DELETE FROM oidc_states WHERE state = $1 AND provider = $2 AND created_at > now() - interval '15 minutes'
         RETURNING code_verifier, invitation_id",
    )
    .bind(state)
    .bind(&provider.id)
    .fetch_optional(&ctx.pool)
    .await
    .map_err(|e| ApiError::from(crate::store::map_sqlx_error(e)))?;
    let Some((verifier, invitation_id)) = row else {
        return Err(ApiError::unauthenticated("The sign-in expired or was already used. Start it again."));
    };
    let endpoints = endpoints(provider).await?;
    let token: serde_json::Value = client()?
        .post(&endpoints.token)
        .header("Accept", "application/json")
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", redirect_uri(config, &provider.id).as_str()),
            ("client_id", provider.client_id.as_str()),
            ("client_secret", provider.client_secret.as_str()),
            ("code_verifier", verifier.as_str()),
        ])
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|_| ApiError::unauthenticated("The identity provider refused the sign-in."))?
        .json()
        .await
        .map_err(|_| unavailable("The identity provider answered something unexpected."))?;
    let access = token["access_token"]
        .as_str()
        .ok_or_else(|| ApiError::unauthenticated("The identity provider returned no access token."))?;
    let profile = fetch_profile(provider, &endpoints, access).await?;
    resolve(ctx, config, &provider.id, &profile, invitation_id).await
}

/// Decide who the profile is. See the module comment for the two ways a stranger gets in.
pub async fn resolve(
    ctx: &ServiceContext,
    config: &Config,
    provider: &str,
    profile: &Profile,
    invitation_id: Option<Uuid>,
) -> Result<(Uuid, String), ApiError> {
    let db = |e: sqlx::Error| ApiError::from(crate::store::map_sqlx_error(e));
    // 1. A known identity.
    let known: Option<(Uuid, String, String)> = sqlx::query_as(
        "SELECT u.id, u.display_name, u.status FROM user_identities i JOIN users u ON u.id = i.user_id
          WHERE i.provider = $1 AND i.subject = $2",
    )
    .bind(provider)
    .bind(&profile.subject)
    .fetch_optional(&ctx.pool)
    .await
    .map_err(db)?;
    if let Some((id, name, status)) = known {
        if status != "active" {
            return Err(ApiError::unauthenticated("This account is disabled."));
        }
        if let Some(invitation) = invitation_id {
            accept_invitation(ctx, invitation, id, &profile.email).await?;
        }
        return Ok((id, name));
    }
    if !profile.email_verified {
        return Err(ApiError::unauthenticated("The identity provider has not verified this email address."));
    }
    // 2. A stranger with an invitation.
    if let Some(invitation) = invitation_id {
        let existing: Option<(Uuid, String)> = sqlx::query_as("SELECT id, display_name FROM users WHERE lower(email) = $1")
            .bind(&profile.email)
            .fetch_optional(&ctx.pool)
            .await
            .map_err(db)?;
        let (user_id, name) = match existing {
            Some(found) => found,
            None => {
                let org: Uuid = sqlx::query_scalar("SELECT organization_id FROM invitations WHERE id = $1")
                    .bind(invitation)
                    .fetch_one(&ctx.pool)
                    .await
                    .map_err(db)?;
                let id: Uuid = sqlx::query_scalar(
                    "INSERT INTO users (organization_id, email, display_name, password_hash, role)
                     VALUES ($1, $2, $3, '!identity-provider', 'viewer') RETURNING id",
                )
                .bind(org)
                .bind(&profile.email)
                .bind(&profile.name)
                .fetch_one(&ctx.pool)
                .await
                .map_err(db)?;
                (id, profile.name.clone())
            }
        };
        accept_invitation(ctx, invitation, user_id, &profile.email).await?;
        link(ctx, provider, profile, user_id).await?;
        return Ok((user_id, name));
    }
    // 3. A stranger without one.
    if config.deploy_mode != DeployMode::Saas {
        return Err(ApiError::unauthenticated("This installation is by invitation. Ask an administrator for a link."));
    }
    let taken: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users WHERE lower(email) = $1)")
        .bind(&profile.email)
        .fetch_one(&ctx.pool)
        .await
        .map_err(db)?;
    if taken {
        return Err(ApiError::unauthenticated(
            "An account with this email exists. Sign in with its password and link the provider, or ask for an invitation.",
        ));
    }
    let platform: Uuid = sqlx::query_scalar("SELECT id FROM organizations WHERE kind = 'platform' ORDER BY created_at LIMIT 1")
        .fetch_one(&ctx.pool)
        .await
        .map_err(db)?;
    let user_id: Uuid = sqlx::query_scalar(
        "INSERT INTO users (organization_id, email, display_name, password_hash, role)
         VALUES ($1, $2, $3, '!identity-provider', 'admin') RETURNING id",
    )
    .bind(platform)
    .bind(&profile.email)
    .bind(&profile.name)
    .fetch_one(&ctx.pool)
    .await
    .map_err(db)?;
    let company = format!("{}'s workspace", profile.name);
    tenancy_service::onboard(&ctx.pool, &company, Some(platform), OrganizationKind::Customer, Some(user_id)).await?;
    link(ctx, provider, profile, user_id).await?;
    Ok((user_id, profile.name.clone()))
}

async fn link(ctx: &ServiceContext, provider: &str, profile: &Profile, user_id: Uuid) -> Result<(), ApiError> {
    sqlx::query("INSERT INTO user_identities (provider, subject, user_id, email) VALUES ($1, $2, $3, $4) ON CONFLICT DO NOTHING")
        .bind(provider)
        .bind(&profile.subject)
        .bind(user_id)
        .bind(&profile.email)
        .execute(&ctx.pool)
        .await
        .map_err(|e| ApiError::from(crate::store::map_sqlx_error(e)))?;
    Ok(())
}

/// Use an invitation: membership with its role (never lowering an existing one) and, when it
/// names a sandbox, the grant. An invitation for a particular email is refused to anyone
/// else, and an invitation is used once.
pub async fn accept_invitation(ctx: &ServiceContext, invitation_id: Uuid, user_id: Uuid, email: &str) -> Result<(), ApiError> {
    let db = |e: sqlx::Error| ApiError::from(crate::store::map_sqlx_error(e));
    let mut tx = ctx.pool.begin().await.map_err(db)?;
    let row: Option<(Uuid, String, Option<Uuid>, Option<String>, Uuid)> = sqlx::query_as(
        "SELECT environment_id, role, sandbox_id, email, organization_id FROM invitations
          WHERE id = $1 AND accepted_at IS NULL AND expires_at > now() FOR UPDATE",
    )
    .bind(invitation_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(db)?;
    let Some((environment_id, role, sandbox_id, wanted, _org)) = row else {
        return Err(ApiError::not_found(ErrorCode::SANDBOX_NOT_FOUND, "invitation"));
    };
    if wanted.as_deref().is_some_and(|w| w != email) {
        return Err(ApiError::forbidden("invitation.other_email"));
    }
    sqlx::query(
        "INSERT INTO user_scopes (user_id, environment_id, role) VALUES ($1, $2, $3)
         ON CONFLICT (user_id, environment_id) DO UPDATE
            SET role = CASE WHEN nb_role_rank(EXCLUDED.role) > nb_role_rank(user_scopes.role)
                            THEN EXCLUDED.role ELSE user_scopes.role END",
    )
    .bind(user_id)
    .bind(environment_id)
    .bind(&role)
    .execute(&mut *tx)
    .await
    .map_err(db)?;
    if let Some(sandbox_id) = sandbox_id {
        sqlx::query("SELECT set_config('app.environment_id', $1, true)")
            .bind(environment_id.to_string())
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        sqlx::query(
            "INSERT INTO sandbox_grants (sandbox_id, user_id, environment_id) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
        )
        .bind(sandbox_id)
        .bind(user_id)
        .bind(environment_id)
        .execute(&mut *tx)
        .await
        .map_err(db)?;
    }
    sqlx::query("UPDATE invitations SET accepted_at = now(), accepted_by = $2 WHERE id = $1")
        .bind(invitation_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await
        .map_err(db)?;
    tx.commit().await.map_err(db)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_pkce_challenge_matches_the_rfc_example() {
        // RFC 7636, appendix B.
        assert_eq!(
            challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn an_oidc_profile_is_read_with_a_verified_email() {
        let user = json!({"sub": "1234", "email": "Ada@Example.com", "email_verified": true, "name": "Ada"});
        let profile = parse_profile("oidc", &user, None).expect("profile");
        assert_eq!(profile.subject, "1234");
        assert_eq!(profile.email, "ada@example.com");
        assert!(profile.email_verified);
    }

    #[test]
    fn an_unverified_email_is_reported_as_such() {
        let user = json!({"sub": "1", "email": "a@b.co", "email_verified": "false"});
        assert!(!parse_profile("oidc", &user, None).expect("profile").email_verified);
        let user = json!({"sub": "1", "email": "a@b.co"});
        assert!(!parse_profile("oidc", &user, None).expect("profile").email_verified);
    }

    #[test]
    fn a_profile_without_a_subject_is_refused() {
        assert!(parse_profile("oidc", &json!({"email": "a@b.co"}), None).is_none());
        assert!(parse_profile("oidc", &json!({"sub": "", "email": "a@b.co"}), None).is_none());
    }

    #[test]
    fn github_uses_its_numeric_id_and_the_primary_verified_email() {
        let user = json!({"id": 4711, "login": "ada"});
        let emails = json!([
            {"email": "other@x.io", "primary": false, "verified": true},
            {"email": "Ada@X.io", "primary": true, "verified": true}
        ]);
        let profile = parse_profile("github", &user, Some(&emails)).expect("profile");
        assert_eq!(profile.subject, "4711");
        assert_eq!(profile.email, "ada@x.io");
        assert_eq!(profile.name, "ada");
        // No verified primary address: no profile, so no account.
        let unverified = json!([{"email": "a@x.io", "primary": true, "verified": false}]);
        assert!(parse_profile("github", &user, Some(&unverified)).is_none());
    }
}
