//! Turning a period of observation into rule drafts in one step (ADR 0053).
//!
//! Everything that was denied, or that only got through because rules were
//! suspended, becomes one allow draft per destination, so the operator reviews
//! a list instead of retyping hosts.
//!
//! Three decisions shape this module:
//!
//! 1. **The window is bounded and the list is capped.** A day is the default and
//!    the query caps the result, because "everything that was ever blocked" is a
//!    review nobody finishes and a draft table nobody wants.
//! 2. **One draft per destination, not per request.** The store groups by
//!    sandbox, host, port, protocol and channel, so a target that was tried four
//!    hundred times is one line to approve.
//! 3. **A failure on one destination does not lose the others.** A draft that
//!    cannot be written is counted and logged; the operator gets the rest, and
//!    the count says what is missing.

use chrono::{DateTime, Duration, Utc};
use uuid::Uuid;

use super::policy_service;
use super::{require, ServiceContext};
use crate::domain::auth::{Permission, Principal};
use crate::error::ApiError;
use crate::store::postgres::traffic as traffic_store;
use crate::store::TenantScope;

/// How far back a request without `since` looks: one day.
const DEFAULT_HOURS: i64 = 24;

/// The most destinations one call turns into drafts.
const DRAFT_LIMIT: i64 = 200;

/// Check that this principal may draft rules over what it observed.
///
/// Drafting is a policy write. The narrower `SandboxPolicy` permission is enough
/// when the request names the sandbox it writes for: a member may prepare rules
/// for their own box, and an environment-wide sweep stays with an operator.
fn require_draft_permission(
    principal: &Principal,
    sandbox_id: Option<Uuid>,
) -> Result<(), ApiError> {
    if require(principal, Permission::PolicyWrite).is_ok() {
        return Ok(());
    }
    if sandbox_id.is_some() && principal.can(Permission::SandboxPolicy) {
        return Ok(());
    }
    Err(ApiError::forbidden(Permission::PolicyWrite.code()))
}

/// Draft one allow rule per destination that was denied or only observed.
pub async fn drafts_from_observations(
    ctx: &ServiceContext,
    principal: &Principal,
    sandbox_id: Option<Uuid>,
    since: Option<DateTime<Utc>>,
) -> Result<serde_json::Value, ApiError> {
    require_draft_permission(principal, sandbox_id)?;

    let until = Utc::now();
    let since = since.unwrap_or_else(|| until - Duration::hours(DEFAULT_HOURS));
    if since > until {
        return Err(ApiError::validation("`since` must lie in the past."));
    }

    // The read and the drafts are separate transactions on purpose: a draft that
    // fails must not roll back the ones already written, and the source window is
    // a read that gains nothing from being held open across many inserts.
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let targets = traffic_store::denied_targets(&mut scope, since, until, sandbox_id, DRAFT_LIMIT)
        .await
        .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;

    let mut created = 0usize;
    let mut failed = 0usize;
    let mut drafts = Vec::with_capacity(targets.len());
    for target in targets {
        // The decision is stated as a deny so the draft's action is `allow`: the
        // destination was stopped, and the useful review is "let it through".
        let event = serde_json::json!({
            "decision": "deny",
            "host": target.host,
            "port": target.port,
            "protocol": target.protocol,
        });
        match policy_service::draft_from_event(ctx, principal, "network", &event, target.sandbox_id)
            .await
        {
            Ok(draft) => {
                created += 1;
                drafts.push(draft);
            }
            Err(error) => {
                failed += 1;
                tracing::warn!(
                    host = %target.host,
                    port = target.port,
                    error = %error.message,
                    "bulk draft skipped"
                );
            }
        }
    }

    Ok(serde_json::json!({
        "created": created,
        "failed": failed,
        "drafts": drafts,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::auth::{CredentialSource, Role};

    fn principal(role: Role) -> Principal {
        Principal {
            user_id: Uuid::nil(),
            environment_id: Uuid::nil(),
            role,
            display_name: "t".into(),
            via: CredentialSource::Session,
        }
    }

    #[test]
    fn an_environment_wide_sweep_needs_the_policy_write_permission() {
        assert!(require_draft_permission(&principal(Role::Member), None).is_err());
        assert!(require_draft_permission(&principal(Role::Operator), None).is_ok());
        assert!(require_draft_permission(&principal(Role::Admin), None).is_ok());
    }

    #[test]
    fn a_member_may_draft_for_one_named_sandbox() {
        let member = principal(Role::Member);
        // `SandboxPolicy` is a member permission; the named sandbox is what
        // narrows the blast radius of the sweep.
        assert!(require_draft_permission(&member, Some(Uuid::new_v4())).is_ok());
        let refusal = require_draft_permission(&member, None).expect_err("refused");
        assert_eq!(refusal.code, crate::error::ErrorCode::FORBIDDEN);
    }
}
