//! Which models a sandbox may use, and what it sees of them (ADR 0056).
//!
//! Grants are inherited environment → user → sandbox; the effective list feeds
//! the agent configuration and the accounting proxy's model router. An
//! environment or user grant is an administrator's decision, a sandbox grant is
//! its owner's: the console must not let a member widen their own models, which
//! is why the scope decides the permission rather than the route.

use uuid::Uuid;

use super::{require, ServiceContext};
use crate::domain::agent_auth::{AgentKind, RestartPolicy};
use crate::domain::auth::{Permission, Principal, Role};
use crate::domain::model_grant::{EffectiveModel, ModelGrant, NewModelGrant};
use crate::error::{ApiError, ErrorCode};
use crate::store::postgres::model_grants as store;
use crate::store::TenantScope;

/// The grants visible to the caller, optionally filtered.
///
/// An administrator sees every scope; everybody else sees the grants of a
/// sandbox they can read, or their own, because the environment-wide list names
/// models the caller may not use.
pub async fn list(
    ctx: &ServiceContext,
    principal: &Principal,
    sandbox_id: Option<Uuid>,
    user_id: Option<Uuid>,
) -> Result<Vec<ModelGrant>, ApiError> {
    require(principal, Permission::SandboxRead)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    if principal.role != Role::Admin {
        match (sandbox_id, user_id) {
            (Some(id), _) => {
                // Row-level security decides whether this sandbox is the
                // caller's; the read is also what makes "no such sandbox" a 404
                // rather than an empty list that looks like a permission bug.
                crate::store::postgres::sandboxes::get(&mut scope, id)
                    .await
                    .map_err(ApiError::from)?;
            }
            (None, Some(user)) if user == principal.user_id => {}
            _ => return Err(ApiError::forbidden(Permission::SandboxRead.code())),
        }
    }
    let grants = store::list(&mut scope, sandbox_id, user_id)
        .await
        .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(grants)
}

/// Grant a model.
///
/// Environment and user grants are admin-only; a sandbox grant needs write
/// access to that sandbox. The profile must exist in this environment, because a
/// grant that points at another tenant's endpoint would route a sandbox into it.
pub async fn create(
    ctx: &ServiceContext,
    principal: &Principal,
    request: NewModelGrant,
) -> Result<ModelGrant, ApiError> {
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let request = match request.scope.as_str() {
        "environment" => {
            if principal.role != Role::Admin {
                return Err(ApiError::forbidden("model.grant"));
            }
            if request.user_id.is_some() || request.sandbox_id.is_some() {
                return Err(ApiError::validation(
                    "An environment grant names neither a user nor a sandbox.",
                ));
            }
            request
        }
        "user" => {
            if principal.role != Role::Admin {
                return Err(ApiError::forbidden("model.grant"));
            }
            if request.user_id.is_none() || request.sandbox_id.is_some() {
                return Err(ApiError::validation(
                    "A user grant names a user and no sandbox.",
                ));
            }
            request
        }
        "sandbox" => {
            require(principal, Permission::SandboxWrite)?;
            let Some(sandbox_id) = request.sandbox_id else {
                return Err(ApiError::validation("A sandbox grant names a sandbox."));
            };
            if request.user_id.is_some() {
                return Err(ApiError::validation("A sandbox grant names no user."));
            }
            // The read is the permission check: row-level security only returns a
            // sandbox this caller owns or was granted.
            crate::store::postgres::sandboxes::get(&mut scope, sandbox_id)
                .await
                .map_err(ApiError::from)?;
            request
        }
        _ => {
            return Err(ApiError::validation(
                "A grant's scope is environment, user or sandbox.",
            ))
        }
    };
    if !valid_model_name(&request.model_name) {
        return Err(ApiError::validation(
            "A model name is 1 to 120 letters, digits or one of . _ : / @ -.",
        ));
    }
    match crate::store::postgres::providers::get(&mut scope, request.provider_profile_id).await {
        Ok(_) => {}
        Err(crate::error::StoreError::NotFound) => {
            return Err(ApiError::validation("Unknown provider profile."))
        }
        Err(error) => return Err(ApiError::from(error)),
    }

    let grant = store::insert(&mut scope, &request, Some(principal.user_id))
        .await
        .map_err(ApiError::from)?;
    let affected = store::sandboxes_affected(&mut scope, &grant)
        .await
        .map_err(ApiError::from)?;
    crate::store::postgres::audit::record(
        &mut scope,
        crate::store::postgres::audit::NewAuditEvent {
            actor_id: Some(principal.user_id),
            actor_label: &principal.display_name,
            kind: "model.grant",
            subject_type: Some("model_grant"),
            subject_id: Some(grant.id),
            detail: &serde_json::json!({
                "scope": grant.scope,
                "user_id": grant.user_id,
                "sandbox_id": grant.sandbox_id,
                "model_name": grant.model_name,
                "provider_profile_id": grant.provider_profile_id,
                "is_default": grant.is_default,
            }),
            request_id: None,
        },
    )
    .await
    .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;

    apply_change(ctx, principal, principal.environment_id, &affected).await;
    Ok(grant)
}

/// Revoke a grant.
///
/// The row's scope decides who may revoke it, so an administrator's environment
/// grant cannot be removed by the owner of one sandbox.
pub async fn remove(ctx: &ServiceContext, principal: &Principal, id: Uuid) -> Result<(), ApiError> {
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let Some(grant) = store::get(&mut scope, id).await.map_err(ApiError::from)? else {
        return Err(ApiError::not_found(
            ErrorCode::NOT_FOUND,
            "No such model grant.",
        ));
    };
    match grant.scope.as_str() {
        "environment" | "user" => {
            if principal.role != Role::Admin {
                return Err(ApiError::forbidden("model.grant"));
            }
        }
        _ => {
            require(principal, Permission::SandboxWrite)?;
            let Some(sandbox_id) = grant.sandbox_id else {
                return Err(ApiError::not_found(
                    ErrorCode::NOT_FOUND,
                    "No such model grant.",
                ));
            };
            crate::store::postgres::sandboxes::get(&mut scope, sandbox_id)
                .await
                .map_err(ApiError::from)?;
        }
    }

    let affected = store::sandboxes_affected(&mut scope, &grant)
        .await
        .map_err(ApiError::from)?;
    if store::delete(&mut scope, id)
        .await
        .map_err(ApiError::from)?
        .is_none()
    {
        return Err(ApiError::not_found(
            ErrorCode::NOT_FOUND,
            "No such model grant.",
        ));
    }
    crate::store::postgres::audit::record(
        &mut scope,
        crate::store::postgres::audit::NewAuditEvent {
            actor_id: Some(principal.user_id),
            actor_label: &principal.display_name,
            kind: "model.revoke",
            subject_type: Some("model_grant"),
            subject_id: Some(id),
            detail: &serde_json::json!({
                "scope": grant.scope,
                "user_id": grant.user_id,
                "sandbox_id": grant.sandbox_id,
                "model_name": grant.model_name,
            }),
            request_id: None,
        },
    )
    .await
    .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;

    apply_change(ctx, principal, principal.environment_id, &affected).await;
    Ok(())
}

/// The models one sandbox may use, after inheritance.
pub async fn effective_for_sandbox(
    ctx: &ServiceContext,
    principal: &Principal,
    sandbox_id: Uuid,
) -> Result<Vec<EffectiveModel>, ApiError> {
    require(principal, Permission::SandboxRead)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let sandbox = store::sandbox_ref(&mut scope, sandbox_id)
        .await
        .map_err(ApiError::from)?
        .ok_or_else(|| ApiError::not_found(ErrorCode::SANDBOX_NOT_FOUND, "No such sandbox."))?;
    let grants = store::for_sandbox(&mut scope, sandbox_id, sandbox.owner_user_id)
        .await
        .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(crate::domain::model_grant::effective(&grants))
}

/// The models one sandbox may use, looked up by its name (system path).
///
/// A sandbox the name does not resolve to has no grants: this path configures a
/// sandbox that already exists, so an unknown name is an empty list rather than
/// an error that would fail a create.
pub async fn effective_by_name(
    ctx: &ServiceContext,
    environment_id: Uuid,
    sandbox_name: &str,
) -> Result<Vec<EffectiveModel>, ApiError> {
    let mut scope = TenantScope::begin(&ctx.pool, environment_id)
        .await
        .map_err(ApiError::from)?;
    let Some(sandbox) = store::sandbox_ref_by_name(&mut scope, sandbox_name)
        .await
        .map_err(ApiError::from)?
    else {
        return Ok(Vec::new());
    };
    let grants = store::for_sandbox(&mut scope, sandbox.id, sandbox.owner_user_id)
        .await
        .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(crate::domain::model_grant::effective(&grants))
}

/// What the accounting proxy needs to route a sandbox's requests by model.
///
/// The proxy reaches this over the appliance token, so the environment comes
/// from the sandbox name it sent — `nb_sandbox_environment(name)` through
/// [`crate::services::media_service::sandbox_environment`] — and the read then
/// runs under that scope. An unknown name is a 404 and a name that exists in two
/// data centers a 409: either way the request is refused rather than routed into
/// a data center the proxy did not mean.
///
/// The credential is resolved here and travels in the answer, never into the
/// database and never into a log line.
pub async fn routes_for_accounting(
    ctx: &ServiceContext,
    sandbox_name: &str,
) -> Result<serde_json::Value, ApiError> {
    let environment_id =
        crate::services::media_service::sandbox_environment(ctx, sandbox_name).await?;
    let mut scope = TenantScope::begin(&ctx.pool, environment_id)
        .await
        .map_err(ApiError::from)?;
    let Some(sandbox) = store::sandbox_ref_by_name(&mut scope, sandbox_name)
        .await
        .map_err(ApiError::from)?
    else {
        return Err(ApiError::not_found(
            ErrorCode::SANDBOX_NOT_FOUND,
            "No such sandbox in this environment.",
        ));
    };
    let grants = store::for_sandbox(&mut scope, sandbox.id, sandbox.owner_user_id)
        .await
        .map_err(ApiError::from)?;
    let models = crate::domain::model_grant::effective(&grants);

    let mut routes = Vec::with_capacity(models.len());
    for model in &models {
        let profile =
            crate::store::postgres::providers::get(&mut scope, model.provider_profile_id)
                .await
                .map_err(ApiError::from)?;
        let authorization = match profile.secret_ref.as_deref() {
            Some(locator) => crate::services::secret_service::credential(
                ctx,
                environment_id,
                locator,
            )
            .await
            .ok()
            .map(|key| format!("Bearer {key}")),
            None => None,
        };
        routes.push(serde_json::json!({
            "model_name": model.model_name,
            "upstream_base": upstream_base(&profile.endpoint),
            "upstream_model": model.upstream_model,
            "authorization": authorization,
        }));
    }
    scope.commit().await.map_err(ApiError::from)?;

    let default = models
        .iter()
        .find(|model| model.is_default)
        .map(|model| model.model_name.clone());
    Ok(serde_json::json!({ "default": default, "models": routes }))
}

/// A model name the table's `CHECK` accepts, rejected here so a bad name is a
/// 422 with a message rather than a 500 from a constraint.
fn valid_model_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 120
        && name
            .chars()
            .all(|symbol| symbol.is_ascii_alphanumeric() || matches!(symbol, '.' | '_' | ':' | '/' | '@' | '-'))
}

/// The base URL an upstream is called at: no trailing `/v1`, no trailing slash.
///
/// The proxy appends the family's own path (`/v1/chat/completions` or
/// `/v1/messages`), so a profile that stores its OpenAI base with `/v1` would
/// otherwise produce `/v1/v1/...`.
fn upstream_base(endpoint: &str) -> String {
    let trimmed = endpoint.trim().trim_end_matches('/');
    trimmed.strip_suffix("/v1").unwrap_or(trimmed).to_string()
}

/// Make a change reach the sandboxes it covers, without blocking the request.
///
/// The files of a running sandbox are rewritten (the agent config carries the
/// model list), the console is told, and an agent that reads its config once at
/// start (openchamber, opencode) is restarted when it is idle. A stopped sandbox
/// gets the same files the next time it starts, so nothing is spawned for it.
async fn apply_change(
    ctx: &ServiceContext,
    principal: &Principal,
    environment_id: Uuid,
    affected: &[Uuid],
) {
    for sandbox_id in affected.iter().copied() {
        let facts = match read_sandbox(ctx, environment_id, sandbox_id).await {
            Ok(Some(facts)) => facts,
            Ok(None) => continue,
            Err(error) => {
                tracing::warn!(
                    sandbox = %sandbox_id,
                    reason = %error,
                    "model grant change could not read a sandbox"
                );
                continue;
            }
        };
        if facts.status != "running" {
            continue;
        }

        let writer = ctx.clone();
        tokio::spawn(async move {
            if let Err(error) =
                crate::services::sandbox_service::write_agent_files(&writer, environment_id, sandbox_id)
                    .await
            {
                tracing::warn!(
                    code = error.code.0,
                    "agent files not rewritten after a model grant change"
                );
            }
        });

        ctx.bus.publish(crate::domain::telemetry::LiveEvent::ModelsChanged {
            sandbox_id: Some(sandbox_id),
            environment_id,
            detail: serde_json::json!({}),
        });

        if matches!(facts.pack_key.as_str(), "openchamber" | "opencode") {
            let restarter = ctx.clone();
            let caller = principal.clone();
            tokio::spawn(async move {
                if let Err(error) = crate::services::agent_auth_service::restart(
                    &restarter,
                    &caller,
                    sandbox_id,
                    AgentKind::Opencode,
                    RestartPolicy::Idle,
                )
                .await
                {
                    tracing::warn!(
                        code = error.code.0,
                        "agent restart after a model grant change failed"
                    );
                }
            });
        }
    }
}

/// Read one sandbox's facts in a system scope; the caller has already proven
/// access, and the effect reaches files rather than the caller's own view.
async fn read_sandbox(
    ctx: &ServiceContext,
    environment_id: Uuid,
    sandbox_id: Uuid,
) -> Result<Option<store::SandboxRef>, String> {
    let mut scope = TenantScope::begin(&ctx.pool, environment_id)
        .await
        .map_err(|error| error.to_string())?;
    let facts = store::sandbox_ref(&mut scope, sandbox_id)
        .await
        .map_err(|error| error.to_string())?;
    scope.commit().await.map_err(|error| error.to_string())?;
    Ok(facts)
}
