//! DLP exceptions: create, list, delete, and the check the content filter asks.
//!
//! An exception is matched by the salted fingerprint, never by the value, so the
//! table cannot hold content. Whoever may grant one is a policy decision: an
//! administrator always, a sandbox owner only when the environment allows it.
//!
//! Three rules keep the feature narrow rather than a general "off" switch:
//!
//! * An exception is always made from a finding, so it starts with real evidence
//!   and its reason is attached to something that happened.
//! * Only an administrator may widen it to the whole environment; an owner's
//!   exception stays inside the sandbox they are responsible for.
//! * `block` is not a possible action. An exception relaxes a decision; it never
//!   turns one finding into a different, stricter rule for everything else.

use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::json;
use utoipa::ToSchema;
use uuid::Uuid;

use super::access_control::require;
use super::privacy_service::effective_settings;
use super::ServiceContext;
use crate::domain::auth::{Permission, Principal, Role};
use crate::domain::privacy::Action;
use crate::domain::privacy_exception::PrivacyException;
use crate::error::{ApiError, ErrorCode};
use crate::store::postgres::privacy_exceptions as store;
use crate::store::TenantScope;

/// Body of `POST /privacy/exceptions`.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct NewExceptionRequest {
    pub incident_id: Uuid,
    /// `value` (this value only) or `detector` (any value of this detector).
    pub scope: String,
    /// true: only the incident's sandbox; false: the whole environment (admin only).
    pub sandbox_only: bool,
    /// `allow`, `replace` or `pseudonymize`.
    pub action: String,
    /// 15, 60, 1440, 10080, or `None` for no end.
    pub minutes: Option<i64>,
    #[serde(default)]
    pub reason: String,
}

/// The lifetimes a client may ask for: a quarter hour to a week, or no end.
///
/// A closed set rather than a free number: "until Friday at some point" is a
/// decision nobody can audit, and every value here is one the console offers.
const ALLOWED_MINUTES: [i64; 4] = [15, 60, 1440, 10080];

/// The longest reason the column accepts.
const MAX_REASON_CHARS: usize = 500;

/// Exceptions that apply to one sandbox now: its own and the environment's.
pub async fn active_for(
    scope: &mut TenantScope<'_>,
    sandbox_id: Uuid,
) -> Result<Vec<PrivacyException>, ApiError> {
    store::active(scope, sandbox_id)
        .await
        .map_err(ApiError::from)
}

/// Create an exception from one incident.
///
/// The incident supplies the sandbox, the detector and the fingerprint; the body
/// supplies only what the person decided. Nothing here reads the matched value,
/// because nothing stores it.
pub async fn create(
    ctx: &ServiceContext,
    principal: &Principal,
    request: NewExceptionRequest,
) -> Result<PrivacyException, ApiError> {
    require(principal, Permission::SandboxRead)?;
    let (scope_kind, action) = validate(&request)?;

    // Read the evidence and decide whether this caller may speak for it. One
    // transaction, committed before `effective_settings` opens its own.
    let (basis, environment_id) = {
        let mut scope = TenantScope::begin_for(&ctx.pool, principal)
            .await
            .map_err(ApiError::from)?;
        let Some(basis) = store::incident_for_exception(&mut scope, request.incident_id)
            .await
            .map_err(ApiError::from)?
        else {
            return Err(incident_not_found());
        };
        let is_admin = principal.role == Role::Admin;
        // Not being the owner is a 404, not a 403: whether somebody else's
        // sandbox has this finding is itself information a member may not have.
        if !is_admin && basis.owner_user_id != Some(principal.user_id) {
            return Err(incident_not_found());
        }
        if !is_admin && !request.sandbox_only {
            return Err(ApiError::forbidden("privacy.exception"));
        }
        let environment_id = scope.environment_id();
        scope.commit().await.map_err(ApiError::from)?;
        (basis, environment_id)
    };

    if principal.role != Role::Admin {
        let settings = effective_settings(ctx, principal, Some(basis.sandbox_id)).await?;
        if !settings.owners_may_grant_exceptions {
            return Err(ApiError::forbidden("privacy.exception"));
        }
    }

    let fingerprint = match scope_kind {
        Scope::Value => Some(basis.value_fingerprint.clone().ok_or_else(|| {
            ApiError::validation(
                "This finding has no fingerprint; allow the detector instead.",
            )
        })?),
        Scope::Detector => None,
    };

    let now = Utc::now();
    let exception = PrivacyException {
        id: Uuid::new_v4(),
        environment_id,
        sandbox_id: request.sandbox_only.then_some(basis.sandbox_id),
        detector: basis.detector,
        value_fingerprint: fingerprint,
        // An exception made from one finding covers that detector from every
        // destination; narrowing by host is a separate decision the body cannot
        // make yet.
        destination: None,
        action,
        expires_at: expires_at(request.minutes, now),
        reason: request.reason.trim().to_string(),
        created_by: Some(principal.user_id),
        created_at: now,
    };

    let detail = json!({
        "detector": exception.detector.as_str(),
        "scope": match scope_kind {
            Scope::Value => "value",
            Scope::Detector => "detector",
        },
        "action": exception.action.as_str(),
        "minutes": request.minutes,
        "sandbox_id": exception.sandbox_id,
    });
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    store::insert(&mut scope, &exception)
        .await
        .map_err(ApiError::from)?;
    crate::store::postgres::audit::record(
        &mut scope,
        crate::store::postgres::audit::NewAuditEvent {
            actor_id: Some(principal.user_id),
            actor_label: &principal.display_name,
            kind: "privacy.exception",
            subject_type: Some(if exception.sandbox_id.is_some() {
                "sandbox"
            } else {
                "environment"
            }),
            subject_id: Some(exception.sandbox_id.unwrap_or(environment_id)),
            detail: &detail,
            request_id: None,
        },
    )
    .await
    .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(exception)
}

/// The exceptions a principal may see.
///
/// An administrator sees the environment, optionally one sandbox; everybody else
/// sees the rows of the sandboxes they own, which is the same rule the console
/// applies to the sandbox list.
pub async fn list(
    ctx: &ServiceContext,
    principal: &Principal,
    sandbox_id: Option<Uuid>,
) -> Result<Vec<PrivacyException>, ApiError> {
    require(principal, Permission::SandboxRead)?;
    let owner = (principal.role != Role::Admin).then_some(principal.user_id);
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let exceptions = store::list(&mut scope, sandbox_id, owner)
        .await
        .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(exceptions)
}

/// Remove one exception.
///
/// The creator may take their own decision back; an administrator may take any.
/// Anyone else gets the same 404 as for a row that does not exist, so the
/// endpoint cannot be used to probe for exceptions.
pub async fn delete(
    ctx: &ServiceContext,
    principal: &Principal,
    id: Uuid,
) -> Result<(), ApiError> {
    require(principal, Permission::SandboxRead)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let Some(exception) = store::get(&mut scope, id)
        .await
        .map_err(ApiError::from)?
    else {
        return Err(exception_not_found());
    };
    if principal.role != Role::Admin && exception.created_by != Some(principal.user_id) {
        return Err(exception_not_found());
    }
    if !store::delete(&mut scope, id)
        .await
        .map_err(ApiError::from)?
    {
        return Err(exception_not_found());
    }
    let detail = json!({
        "detector": exception.detector.as_str(),
        "sandbox_id": exception.sandbox_id,
    });
    crate::store::postgres::audit::record(
        &mut scope,
        crate::store::postgres::audit::NewAuditEvent {
            actor_id: Some(principal.user_id),
            actor_label: &principal.display_name,
            kind: "privacy.exception.delete",
            subject_type: Some(if exception.sandbox_id.is_some() {
                "sandbox"
            } else {
                "environment"
            }),
            subject_id: Some(exception.sandbox_id.unwrap_or(exception.environment_id)),
            detail: &detail,
            request_id: None,
        },
    )
    .await
    .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(())
}

/// How wide the exception is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scope {
    /// Exactly this value, by fingerprint.
    Value,
    /// Every value of this detector.
    Detector,
}

/// Check the body and read its two spellings.
///
/// Everything a client can get wrong is a 422 with a message that names the
/// field, rather than a database error or a silently changed decision.
fn validate(request: &NewExceptionRequest) -> Result<(Scope, Action), ApiError> {
    let scope = match request.scope.as_str() {
        "value" => Scope::Value,
        "detector" => Scope::Detector,
        _ => {
            return Err(ApiError::validation(
                "The scope must be 'value' or 'detector'.",
            ));
        }
    };
    let action = Action::parse(&request.action).ok_or_else(|| {
        ApiError::validation("The action must be 'allow', 'replace' or 'pseudonymize'.")
    })?;
    if !matches!(action, Action::Allow | Action::Replace | Action::Pseudonymize) {
        return Err(ApiError::validation(
            "The action must be 'allow', 'replace' or 'pseudonymize'.",
        ));
    }
    if request
        .minutes
        .is_some_and(|minutes| !ALLOWED_MINUTES.contains(&minutes))
    {
        return Err(ApiError::validation(
            "The minutes must be 15, 60, 1440, 10080 or absent.",
        ));
    }
    if request.reason.chars().count() > MAX_REASON_CHARS {
        return Err(ApiError::validation(
            "The reason must be at most 500 characters.",
        ));
    }
    Ok((scope, action))
}

/// The incident is missing, or the caller may not know it exists.
fn incident_not_found() -> ApiError {
    ApiError::not_found(ErrorCode::NOT_FOUND, "The incident does not exist.")
}

/// The exception is missing, or the caller may not know it exists.
fn exception_not_found() -> ApiError {
    ApiError::not_found(ErrorCode::NOT_FOUND, "The exception does not exist.")
}

/// A moment `minutes` from now, or none for "never".
pub(crate) fn expires_at(minutes: Option<i64>, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    minutes.map(|minutes| now + chrono::Duration::minutes(minutes))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(scope: &str, action: &str, minutes: Option<i64>) -> NewExceptionRequest {
        NewExceptionRequest {
            incident_id: Uuid::new_v4(),
            scope: scope.to_string(),
            sandbox_only: true,
            action: action.to_string(),
            minutes,
            reason: String::new(),
        }
    }

    #[test]
    fn the_three_relaxing_actions_are_accepted() {
        for action in ["allow", "replace", "pseudonymize"] {
            let (_, parsed) = validate(&request("value", action, None)).expect(action);
            assert_eq!(parsed.as_str(), action);
        }
    }

    #[test]
    fn block_is_not_an_exception() {
        assert!(validate(&request("detector", "block", None)).is_err());
        assert!(validate(&request("detector", "shrug", None)).is_err());
    }

    #[test]
    fn an_unknown_scope_is_refused() {
        assert!(validate(&request("everything", "allow", None)).is_err());
    }

    #[test]
    fn only_the_offered_lifetimes_are_accepted() {
        for minutes in [None, Some(15), Some(60), Some(1440), Some(10080)] {
            assert!(validate(&request("detector", "allow", minutes)).is_ok());
        }
        for minutes in [Some(0), Some(30), Some(-15), Some(20_160)] {
            assert!(validate(&request("detector", "allow", minutes)).is_err());
        }
    }

    #[test]
    fn the_reason_has_a_limit() {
        let mut long = request("detector", "allow", None);
        long.reason = "a".repeat(MAX_REASON_CHARS + 1);
        assert!(validate(&long).is_err());

        long.reason = "ä".repeat(MAX_REASON_CHARS);
        assert!(validate(&long).is_ok());
    }

    #[test]
    fn no_minutes_means_no_end() {
        assert!(expires_at(None, Utc::now()).is_none());
        let now = Utc::now();
        assert_eq!(
            expires_at(Some(60), now),
            Some(now + chrono::Duration::minutes(60))
        );
    }
}
